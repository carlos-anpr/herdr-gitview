//! Estado de la aplicación, trabajos en segundo plano y entrada (teclado + ratón).

use crate::config::Config;
use crate::diff::DiffView;
use crate::model::*;
use crate::{ai, git, forge::{self, Forge}, graph};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Graph,
    Mrs,
    Files,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ListId {
    Picker,
    Commits,
    CommitFiles,
    Diff,
    Mrs,
    MrFiles,
    Fs,
    FsHist,
    Ai,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Btn {
    Explain,
    CloseDiff,
    MrState(MrState),
    OpenRepo,
    Quit,
    Help,
    Refresh,
    OpenWeb,
    Back,
    AiMax,
    Reveal,
}

#[derive(Clone, Copy, Debug)]
pub enum Hit {
    Tab(Tab),
    /// Área de lista: fila = off + (y - rect.y).
    List(ListId),
    Btn(Btn),
}

/// Selección + desplazamiento de una lista.
#[derive(Debug, Clone, Default)]
pub struct Nav {
    pub sel: usize,
    pub off: usize,
    /// Alto visible en el último render.
    pub h: usize,
    /// Si true, el render ajusta `off` para que `sel` sea visible.
    pub follow: bool,
}

impl Nav {
    pub fn move_by(&mut self, d: isize, len: usize) {
        if len == 0 {
            self.sel = 0;
            return;
        }
        let s = self.sel as isize + d;
        self.sel = s.clamp(0, len as isize - 1) as usize;
        self.follow = true;
    }
    pub fn set(&mut self, i: usize, len: usize) {
        self.sel = i.min(len.saturating_sub(1));
        self.follow = true;
    }
    pub fn scroll(&mut self, d: isize, len: usize) {
        let max = len.saturating_sub(self.h.max(1));
        self.off = (self.off as isize + d).clamp(0, max as isize) as usize;
        self.follow = false;
    }
    /// Ajusta `off` al alto `h` (llamado en render).
    pub fn fit(&mut self, h: usize, len: usize) {
        self.h = h;
        if len == 0 {
            self.sel = 0;
            self.off = 0;
            return;
        }
        self.sel = self.sel.min(len - 1);
        if self.follow {
            if self.sel < self.off {
                self.off = self.sel;
            } else if h > 0 && self.sel >= self.off + h {
                self.off = self.sel + 1 - h;
            }
        }
        self.off = self.off.min(len.saturating_sub(h.max(1)));
    }
    pub fn page(&self) -> isize {
        self.h.max(2) as isize - 1
    }
}

#[derive(Debug, Clone)]
pub struct FsEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub is_repo: bool,
    pub mark: Option<char>,
}

pub enum AiState {
    /// En curso: inicio y texto parcial recibido hasta ahora.
    Running(Instant, String),
    Done(String),
    Failed(String),
}

#[derive(Clone)]
enum Src {
    Commit(String),
    CommitFile(String, String),
    Working(String),
    WorkingAll,
    Mr(u64),
    Text(String),
}

#[derive(Clone)]
pub struct AiJob {
    pub key: String,
    kind: AiKind,
    pub label: String,
    title: String,
    context: String,
    src: Src,
}

impl AiJob {
    fn request(&self, diff: String) -> AiRequest {
        AiRequest {
            cache_key: self.key.clone(),
            kind: self.kind,
            title: self.title.clone(),
            context: self.context.clone(),
            diff,
        }
    }
}

pub enum Msg {
    Log { generation: u64, skip: usize, res: Result<Vec<Commit>, String> },
    Status(Vec<StatusEntry>),
    Files { sha: String, res: Result<Vec<FileChange>, String> },
    Detail { sha: String, res: Result<CommitDetail, String> },
    Diff { tab: Tab, key: String, title: String, res: Result<String, String> },
    Forge(Option<Forge>),
    Mrs { generation: u64, page: u32, res: Result<Vec<MergeRequest>, String> },
    MrDetail { generation: u64, res: Result<MergeRequest, String> },
    MrDiffs { iid: u64, res: Result<Vec<MrFileDiff>, String> },
    MrCommits { iid: u64, res: Result<Vec<MrCommit>, String> },
    FileLog { path: String, res: Result<Vec<Commit>, String> },
    Ai { key: String, res: Result<String, String> },
    AiPartial { key: String, text: String },
    Picker { base: PathBuf, cands: Vec<Candidate> },
}

pub struct Repo {
    pub root: PathBuf,
    pub name: String,
    pub branch: String,
    pub remote: Option<String>,
    // Grafo
    pub commits: Vec<Commit>,
    pub rows: Vec<GraphRow>,
    pub log_gen: u64,
    pub log_loading: bool,
    pub log_done: bool,
    pub log_err: Option<String>,
    pub status: Vec<StatusEntry>,
    pub status_map: HashMap<String, char>,
    pub g_nav: Nav,
    pub g_files_nav: Nav,
    pub files: HashMap<String, Result<Vec<FileChange>, String>>,
    pub details: HashMap<String, CommitDetail>,
    pub g_diff: Option<DiffView>,
    /// SHA a reseleccionar tras una recarga.
    pub keep_sha: Option<String>,
    // MRs
    pub forge: Option<Option<Forge>>,
    pub mrs: Vec<MergeRequest>,
    pub mr_state: MrState,
    pub mr_page: u32,
    pub mr_loading: bool,
    pub mr_done: bool,
    pub mr_err: Option<String>,
    pub mr_gen: u64,
    pub mr_nav: Nav,
    pub mr_files_nav: Nav,
    pub mr_diffs: HashMap<u64, Result<Vec<MrFileDiff>, String>>,
    pub mr_commits: HashMap<u64, Vec<MrCommit>>,
    pub mr_diff: Option<DiffView>,
    // Ficheros
    pub fs_dir: PathBuf,
    pub fs_entries: Vec<FsEntry>,
    pub fs_nav: Nav,
    pub fs_hist_nav: Nav,
    pub fs_hist: HashMap<String, Result<Vec<Commit>, String>>,
    pub fs_diff: Option<DiffView>,
}

impl Repo {
    pub fn forge_hint(&self) -> Forge {
        self.forge.flatten().unwrap_or_else(|| {
            if self.remote.as_deref().is_some_and(|url| url.contains("github")) {
                Forge::GitHub
            } else {
                Forge::GitLab
            }
        })
    }
}

pub struct Picker {
    pub base: PathBuf,
    pub cands: Vec<Candidate>,
    pub filter: String,
    pub nav: Nav,
}

impl Picker {
    pub fn visible(&self) -> Vec<&Candidate> {
        let f = self.filter.to_lowercase();
        self.cands.iter().filter(|c| f.is_empty() || c.name.to_lowercase().contains(&f)).collect()
    }
}

pub struct App {
    pub cfg: Config,
    pub repo: Option<Repo>,
    pub picker: Option<Picker>,
    pub tab: Tab,
    pub focus: Pane,
    pub ai: HashMap<String, AiState>,
    pub ai_nav: Nav,
    /// La caja de IA ocupa todo el panel derecho.
    pub ai_max: bool,
    pub ai_seen: HashSet<String>,
    pub hits: Vec<(Rect, Hit)>,
    pub diff_h: usize,
    pub help: bool,
    pub toast: Option<(String, Instant)>,
    pub quit: bool,
    pub tick: u64,
    tx: Sender<(u64, Msg)>,
    pub rx: Receiver<(u64, Msg)>,
    /// Cambia al abrir otro repo; los mensajes de un repo anterior se descartan.
    epoch: u64,
    inflight: HashSet<String>,
    pending_diff: Option<String>,
    sel_changed: Option<Instant>,
    last_click: Option<(Instant, u16, u16)>,
}

const DEBOUNCE: Duration = Duration::from_millis(70);

impl App {
    pub fn new(cfg: Config) -> Self {
        let (tx, rx) = channel();
        App {
            cfg,
            repo: None,
            picker: None,
            tab: Tab::Graph,
            focus: Pane::Left,
            ai: HashMap::new(),
            ai_nav: Nav::default(),
            ai_max: false,
            ai_seen: HashSet::new(),
            hits: Vec::new(),
            diff_h: 20,
            help: false,
            toast: None,
            quit: false,
            tick: 0,
            tx,
            rx,
            epoch: 0,
            inflight: HashSet::new(),
            pending_diff: None,
            sel_changed: None,
            last_click: None,
        }
    }

    fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        let epoch = self.epoch;
        std::thread::spawn(move || {
            let _ = tx.send((epoch, f()));
        });
    }

    pub fn toast(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }

    // ───────────────────────────── apertura ─────────────────────────────

    pub fn open_picker(&mut self, base: PathBuf, cands: Vec<Candidate>) {
        self.picker = Some(Picker { base, cands, filter: String::new(), nav: Nav { follow: true, ..Default::default() } });
    }

    /// Abre el repo y deja el explorador en `dir` si está dentro de él.
    pub fn open_repo_at(&mut self, root: PathBuf, dir: &Path) {
        self.open_repo(root);
        if let Some(r) = self.repo.as_mut()
            && dir.starts_with(&r.root)
            && dir != r.root
            && dir.is_dir()
        {
            r.fs_dir = dir.to_path_buf();
            self.read_dir();
        }
    }

    pub fn open_repo(&mut self, root: PathBuf) {
        let name = root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let repo = Repo {
            branch: git::head_branch(&root).unwrap_or_else(|_| "?".into()),
            remote: git::remote_url(&root),
            name,
            commits: Vec::new(),
            rows: Vec::new(),
            log_gen: 0,
            log_loading: false,
            log_done: false,
            log_err: None,
            status: Vec::new(),
            status_map: HashMap::new(),
            g_nav: Nav { follow: true, ..Default::default() },
            g_files_nav: Nav { follow: true, ..Default::default() },
            files: HashMap::new(),
            details: HashMap::new(),
            g_diff: None,
            keep_sha: None,
            forge: None,
            mrs: Vec::new(),
            mr_state: MrState::All,
            mr_page: 0,
            mr_loading: false,
            mr_done: false,
            mr_err: None,
            mr_gen: 0,
            mr_nav: Nav { follow: true, ..Default::default() },
            mr_files_nav: Nav { follow: true, ..Default::default() },
            mr_diffs: HashMap::new(),
            mr_commits: HashMap::new(),
            mr_diff: None,
            fs_dir: root.clone(),
            fs_entries: Vec::new(),
            fs_nav: Nav { follow: true, ..Default::default() },
            fs_hist_nav: Nav { follow: true, ..Default::default() },
            fs_hist: HashMap::new(),
            fs_diff: None,
            root,
        };
        self.repo = Some(repo);
        self.picker = None;
        self.epoch += 1;
        self.pending_diff = None;
        self.tab = Tab::Graph;
        self.focus = Pane::Left;
        self.inflight.clear();
        self.reload_log();
        self.reload_status();
        self.read_dir();
        let root = self.repo.as_ref().unwrap().root.clone();
        self.spawn(move || Msg::Forge(forge::detect(&root)));
        self.sel_changed = Some(Instant::now());
    }

    fn reload_log(&mut self) {
        let keep = self.sel_commit().map(|c| c.id.clone());
        let Some(r) = self.repo.as_mut() else { return };
        r.keep_sha = keep;
        r.log_gen += 1;
        r.log_loading = true;
        r.log_done = false;
        r.log_err = None;
        let (root, generation, n) = (r.root.clone(), r.log_gen, self.cfg.page_size);
        self.spawn(move || Msg::Log { generation, skip: 0, res: git::log(&root, 0, n).map_err(|e| format!("{e:#}")) });
    }

    fn more_log(&mut self) {
        let Some(r) = self.repo.as_mut() else { return };
        if r.log_loading || r.log_done {
            return;
        }
        r.log_loading = true;
        let (root, generation, skip, n) = (r.root.clone(), r.log_gen, r.commits.len(), self.cfg.page_size);
        self.spawn(move || Msg::Log { generation, skip, res: git::log(&root, skip, n).map_err(|e| format!("{e:#}")) });
    }

    fn reload_status(&mut self) {
        let Some(r) = self.repo.as_ref() else { return };
        let root = r.root.clone();
        self.spawn(move || Msg::Status(git::status(&root).unwrap_or_default()));
    }

    fn reload_mrs(&mut self) {
        let Some(r) = self.repo.as_mut() else { return };
        let Some(forge) = r.forge.flatten() else { return; };
        r.mr_gen += 1;
        r.mr_page = 1;
        r.mrs.clear();
        r.mr_done = false;
        r.mr_err = None;
        r.mr_loading = true;
        r.mr_nav = Nav { follow: true, ..Default::default() };
        let (root, generation, st) = (r.root.clone(), r.mr_gen, r.mr_state);
        self.spawn(move || Msg::Mrs { generation, page: 1, res: forge.list_mrs(&root, st, 1).map_err(|e| format!("{e:#}")) });
    }

    fn more_mrs(&mut self) {
        let Some(r) = self.repo.as_mut() else { return };
        if r.mr_loading || r.mr_done || r.forge.flatten().is_none() {
            return;
        }
        let Some(forge) = r.forge.flatten() else { return; };
        r.mr_loading = true;
        r.mr_page += 1;
        let (root, generation, st, page) = (r.root.clone(), r.mr_gen, r.mr_state, r.mr_page);
        self.spawn(move || Msg::Mrs { generation, page, res: forge.list_mrs(&root, st, page).map_err(|e| format!("{e:#}")) });
    }

    pub fn read_dir(&mut self) {
        let Some(r) = self.repo.as_mut() else { return };
        let mut v: Vec<FsEntry> = Vec::new();
        if let Some(parent) = r.fs_dir.parent() {
            v.push(FsEntry { name: "..".into(), path: parent.to_path_buf(), is_dir: true, is_repo: false, mark: None });
        }
        if let Ok(rd) = std::fs::read_dir(&r.fs_dir) {
            let mut items: Vec<FsEntry> = rd
                .flatten()
                .filter(|e| e.file_name() != ".git")
                .map(|e| {
                    let path = e.path();
                    let is_dir = path.is_dir();
                    let mark = mark_for(&r.root, &r.status_map, &path, is_dir);
                    FsEntry {
                        name: e.file_name().to_string_lossy().into_owned(),
                        is_repo: is_dir && path.join(".git").exists(),
                        is_dir,
                        path,
                        mark,
                    }
                })
                .collect();
            items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            v.extend(items);
        }
        r.fs_entries = v;
        r.fs_nav = Nav { follow: true, sel: usize::from(r.fs_entries.len() > 1 && r.fs_dir.parent().is_some()), ..Default::default() };
        self.sel_changed = Some(Instant::now());
    }

    // ─────────────────────────── selección actual ───────────────────────────

    pub fn wip_rows(&self) -> usize {
        self.repo.as_ref().map_or(0, |r| usize::from(!r.status.is_empty()))
    }

    pub fn graph_len(&self) -> usize {
        self.repo.as_ref().map_or(0, |r| r.commits.len() + self.wip_rows())
    }

    /// None = fila WIP; Some(i) = commit i.
    pub fn graph_sel(&self) -> Option<Option<usize>> {
        let r = self.repo.as_ref()?;
        let s = r.g_nav.sel;
        if self.wip_rows() == 1 {
            if s == 0 { Some(None) } else { r.commits.get(s - 1).map(|_| Some(s - 1)) }
        } else {
            r.commits.get(s).map(|_| Some(s))
        }
    }

    pub fn sel_commit(&self) -> Option<&Commit> {
        let r = self.repo.as_ref()?;
        self.graph_sel()?.and_then(|i| r.commits.get(i))
    }

    /// Ficheros del commit/WIP seleccionado (None = cargando).
    pub fn graph_files(&self) -> Option<Result<Vec<FileChange>, String>> {
        let r = self.repo.as_ref()?;
        match self.graph_sel()? {
            None => Some(Ok(r
                .status
                .iter()
                .map(|s| FileChange {
                    path: s.path.clone(),
                    old_path: s.old_path.clone(),
                    status: if s.worktree == '?' { '?' } else if s.worktree != ' ' { s.worktree } else { s.index },
                    additions: 0,
                    deletions: 0,
                    binary: false,
                })
                .collect())),
            Some(i) => r.files.get(&r.commits[i].id).cloned(),
        }
    }

    pub fn sel_mr(&self) -> Option<&MergeRequest> {
        let r = self.repo.as_ref()?;
        r.mrs.get(r.mr_nav.sel)
    }

    pub fn sel_fs(&self) -> Option<&FsEntry> {
        let r = self.repo.as_ref()?;
        r.fs_entries.get(r.fs_nav.sel)
    }

    /// Ruta relativa al repo del fichero seleccionado en el explorador (si está dentro y es fichero).
    pub fn sel_fs_rel(&self) -> Option<String> {
        let r = self.repo.as_ref()?;
        let e = self.sel_fs()?;
        if e.is_dir {
            return None;
        }
        e.path.strip_prefix(&r.root).ok().map(|p| p.to_string_lossy().into_owned())
    }

    /// Filas del historial del fichero: (wip?, commits).
    pub fn fs_hist_rows(&self) -> (bool, Option<&Result<Vec<Commit>, String>>) {
        let Some(r) = self.repo.as_ref() else { return (false, None) };
        let Some(rel) = self.sel_fs_rel() else { return (false, None) };
        (r.status_map.contains_key(&rel), r.fs_hist.get(&rel))
    }

    fn fs_hist_len(&self) -> usize {
        let (wip, h) = self.fs_hist_rows();
        usize::from(wip) + h.and_then(|h| h.as_ref().ok()).map_or(0, |v| v.len())
    }

    pub fn diff_mut(&mut self) -> Option<&mut DiffView> {
        let r = self.repo.as_mut()?;
        match self.tab {
            Tab::Graph => r.g_diff.as_mut(),
            Tab::Mrs => r.mr_diff.as_mut(),
            Tab::Files => r.fs_diff.as_mut(),
        }
    }

    pub fn diff(&self) -> Option<&DiffView> {
        let r = self.repo.as_ref()?;
        match self.tab {
            Tab::Graph => r.g_diff.as_ref(),
            Tab::Mrs => r.mr_diff.as_ref(),
            Tab::Files => r.fs_diff.as_ref(),
        }
    }

    fn close_diff(&mut self) -> bool {
        let Some(r) = self.repo.as_mut() else { return false };
        let d = match self.tab {
            Tab::Graph => &mut r.g_diff,
            Tab::Mrs => &mut r.mr_diff,
            Tab::Files => &mut r.fs_diff,
        };
        self.pending_diff = None;
        d.take().is_some()
    }

    // ───────────────────────────────── IA ─────────────────────────────────

    /// Qué explicaría la IA con la selección actual.
    pub fn ai_job(&self) -> Option<AiJob> {
        let r = self.repo.as_ref()?;
        match self.tab {
            Tab::Graph => {
                let sel = self.graph_sel()?;
                let on_file = self.focus == Pane::Right || r.g_diff.is_some();
                if on_file {
                    let files = self.graph_files()?.ok()?;
                    let f = files.get(r.g_files_nav.sel)?;
                    return Some(match sel {
                        None => {
                            let stamp = std::fs::metadata(r.root.join(&f.path))
                                .and_then(|m| m.modified())
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map_or(0, |d| d.as_secs());
                            AiJob {
                                key: format!("wipfile:{}:{}:{stamp}", r.root.display(), f.path),
                                kind: AiKind::File,
                                label: format!("fichero {} (sin commit)", f.path),
                                title: f.path.clone(),
                                context: format!("Cambios sin commit en {}", r.name),
                                src: Src::Working(f.path.clone()),
                            }
                        }
                        Some(i) => {
                            let c = &r.commits[i];
                            AiJob {
                                key: format!("file:{}:{}", c.id, f.path),
                                kind: AiKind::File,
                                label: format!("fichero {}", f.path),
                                title: f.path.clone(),
                                context: format!("Commit {} — {}\nAutor: {}", c.short, c.subject, c.author),
                                src: Src::CommitFile(c.id.clone(), f.path.clone()),
                            }
                        }
                    });
                }
                Some(match sel {
                    None => {
                        let mut h: u64 = 0xcbf29ce484222325;
                        for s in &r.status {
                            for b in s.path.bytes().chain([s.index as u8, s.worktree as u8]) {
                                h = (h ^ b as u64).wrapping_mul(0x100000001b3);
                            }
                        }
                        AiJob {
                            key: format!("wip:{}:{h:x}:{}", r.root.display(), crate::text::now() / 300),
                            kind: AiKind::Commit,
                            label: "cambios sin commit".into(),
                            title: format!("Cambios sin commit en {}", r.name),
                            context: format!(
                                "Rama {}\nFicheros:\n{}",
                                r.branch,
                                r.status.iter().map(|s| format!("{}{} {}", s.index, s.worktree, s.path)).collect::<Vec<_>>().join("\n")
                            ),
                            src: Src::WorkingAll,
                        }
                    }
                    Some(i) => {
                        let c = &r.commits[i];
                        let msg = r.details.get(&c.id).map(|d| d.message.clone()).unwrap_or_else(|| c.subject.clone());
                        let files = r
                            .files
                            .get(&c.id)
                            .and_then(|f| f.as_ref().ok())
                            .map(|v| v.iter().map(|f| format!("{} {} (+{} -{})", f.status, f.path, f.additions, f.deletions)).collect::<Vec<_>>().join("\n"))
                            .unwrap_or_default();
                        AiJob {
                            key: format!("commit:{}", c.id),
                            kind: AiKind::Commit,
                            label: format!("commit {}", c.short),
                            title: c.subject.clone(),
                            context: format!("Commit {}\nAutor: {}\nMensaje:\n{msg}\nFicheros:\n{files}", c.short, c.author),
                            src: Src::Commit(c.id.clone()),
                        }
                    }
                })
            }
            Tab::Mrs => {
                let forge = r.forge.flatten()?;
                let (short, prefix) = (forge.short(), forge.id_prefix());
                let mr = self.sel_mr()?;
                let remote = r.remote.clone().unwrap_or_default();
                let ver = mr.sha.clone().unwrap_or_else(|| mr.updated_at.to_string());
                let on_file = self.focus == Pane::Right || r.mr_diff.is_some();
                if on_file {
                    let diffs = r.mr_diffs.get(&mr.iid)?.as_ref().ok()?;
                    let f = diffs.get(r.mr_files_nav.sel)?;
                    return Some(AiJob {
                        key: format!("mrfile:{remote}:{}:{ver}:{}", mr.iid, f.new_path),
                        kind: AiKind::File,
                        label: format!("fichero {} ({prefix}{})", f.new_path, mr.iid),
                        title: f.new_path.clone(),
                        context: format!("{short} {prefix}{} — {}\n{} → {}\n{}", mr.iid, mr.title, mr.source_branch, mr.target_branch, clip(&mr.description, 1500)),
                        src: Src::Text(format!("--- a/{}\n+++ b/{}\n{}", f.old_path, f.new_path, f.diff)),
                    });
                }
                let commits = r
                    .mr_commits
                    .get(&mr.iid)
                    .map(|v| v.iter().map(|c| format!("- {}", c.title)).collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default();
                Some(AiJob {
                    key: format!("mr:{remote}:{}:{ver}", mr.iid),
                    kind: AiKind::MergeRequest,
                    label: format!("{short} {prefix}{}", mr.iid),
                    title: mr.title.clone(),
                    context: format!(
                        "{short} {prefix}{} ({})\nAutor: {}\nRamas: {} → {}\nDescripción:\n{}\nCommits:\n{commits}",
                        mr.iid,
                        mr.state,
                        mr.author,
                        mr.source_branch,
                        mr.target_branch,
                        clip(&mr.description, 3000)
                    ),
                    src: Src::Mr(mr.iid),
                })
            }
            Tab::Files => {
                let rel = self.sel_fs_rel()?;
                let (wip, hist) = self.fs_hist_rows();
                let i = r.fs_hist_nav.sel;
                if wip && i == 0 {
                    return Some(AiJob {
                        key: format!("wipfile:{}:{rel}:{}", r.root.display(), crate::text::now() / 60),
                        kind: AiKind::File,
                        label: format!("fichero {rel} (sin commit)"),
                        title: rel.clone(),
                        context: format!("Cambios sin commit en {}", r.name),
                        src: Src::Working(rel),
                    });
                }
                let c = hist?.as_ref().ok()?.get(i - usize::from(wip))?;
                Some(AiJob {
                    key: format!("file:{}:{rel}", c.id),
                    kind: AiKind::File,
                    label: format!("{rel} @ {}", c.short),
                    title: rel.clone(),
                    context: format!("Commit {} — {}\nAutor: {}", c.short, c.subject, c.author),
                    src: Src::CommitFile(c.id.clone(), rel),
                })
            }
        }
    }

    /// Estado IA de la selección actual (consulta caché en disco la primera vez).
    pub fn current_ai(&mut self) -> Option<(AiJob, Option<&AiState>)> {
        let job = self.ai_job()?;
        if !self.ai.contains_key(&job.key) && self.ai_seen.insert(job.key.clone())
            && let Some(txt) = ai::cached(&job.request(String::new()))
        {
            self.ai.insert(job.key.clone(), AiState::Done(txt));
        }
        let st = self.ai.get(&job.key);
        Some((job, st))
    }

    fn explain(&mut self, force: bool) {
        let Some(job) = self.ai_job() else {
            self.toast("Nada que explicar aquí");
            return;
        };
        if matches!(self.ai.get(&job.key), Some(AiState::Running(..))) || (!force && matches!(self.ai.get(&job.key), Some(AiState::Done(_)))) {
            return;
        }
        if !force
            && let Some(txt) = ai::cached(&job.request(String::new()))
        {
            self.ai.insert(job.key.clone(), AiState::Done(txt));
            return;
        }
        let Some(root) = self.repo.as_ref().map(|r| r.root.clone()) else { return };
        self.ai.insert(job.key.clone(), AiState::Running(Instant::now(), String::new()));
        self.ai_nav = Nav::default();
        let forge = self.repo.as_ref().and_then(|r| r.forge.flatten());
        let cfg = self.cfg.clone();
        let max = cfg.ai_max_diff_bytes;
        let tx = self.tx.clone();
        self.spawn(move || {
            let diff: Result<String, String> = match &job.src {
                Src::Commit(sha) => git::commit_diff(&root, sha, max * 2).map_err(|e| format!("{e:#}")),
                Src::CommitFile(sha, p) => git::file_diff(&root, sha, p).map_err(|e| format!("{e:#}")),
                Src::Working(p) => working_both(&root, p),
                Src::WorkingAll => git_out(&root, &["diff", "HEAD", "-M"]),
                Src::Mr(iid) => forge.ok_or_else(|| "forja no disponible".to_owned()).and_then(|forge| forge.mr_diffs(&root, *iid).map_err(|e| format!("{e:#}"))).map(|v| {
                    v.iter().map(|f| format!("--- a/{}\n+++ b/{}\n{}", f.old_path, f.new_path, f.diff)).collect::<Vec<_>>().join("\n")
                }),
                Src::Text(t) => Ok(t.clone()),
            };
            let mut last = Instant::now();
            let key = job.key.clone();
            let mut on_text = |t: &str| {
                // Como mucho ~12 repintados por segundo.
                if last.elapsed() >= Duration::from_millis(80) {
                    last = Instant::now();
                    let _ = tx.send((0, Msg::AiPartial { key: key.clone(), text: t.to_string() }));
                }
            };
            let res = diff.and_then(|d| ai::explain_live(&cfg, &job.request(d), force, &mut on_text).map_err(|e| format!("{e:#}")));
            Msg::Ai { key: job.key.clone(), res }
        });
    }

    // ──────────────────────────── carga perezosa ────────────────────────────

    /// Llamado en cada iteración: lanza cargas pendientes tras el debounce.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if let Some((_, t)) = &self.toast
            && t.elapsed() > Duration::from_secs(3)
        {
            self.toast = None;
        }
        let Some(t) = self.sel_changed else { return };
        if t.elapsed() < DEBOUNCE {
            return;
        }
        self.sel_changed = None;
        self.load_for_selection();
    }

    fn load_for_selection(&mut self) {
        let Some(r) = self.repo.as_ref() else { return };
        let root = r.root.clone();
        match self.tab {
            Tab::Graph => {
                // Paginación anticipada.
                if r.g_nav.sel + 80 >= r.commits.len() {
                    self.more_log();
                }
                let Some(c) = self.sel_commit().cloned() else { return };
                let r = self.repo.as_ref().unwrap();
                if !r.files.contains_key(&c.id) && self.inflight.insert(format!("files:{}", c.id)) {
                    let (root, sha) = (root.clone(), c.id.clone());
                    self.spawn(move || Msg::Files { res: git::commit_files(&root, &sha).map_err(|e| format!("{e:#}")), sha });
                }
                if !r.details.contains_key(&c.id) && self.inflight.insert(format!("detail:{}", c.id)) {
                    let sha = c.id.clone();
                    self.spawn(move || Msg::Detail { res: git::commit_detail(&root, &sha).map_err(|e| format!("{e:#}")), sha });
                }
            }
            Tab::Mrs => {
                if r.forge.flatten().is_some() && r.mrs.is_empty() && !r.mr_loading && r.mr_err.is_none() && !r.mr_done {
                    self.reload_mrs();
                    return;
                }
                if r.mr_nav.sel + 10 >= r.mrs.len() {
                    self.more_mrs();
                }
                let Some(iid) = self.sel_mr().map(|m| m.iid) else { return };
                let r = self.repo.as_ref().unwrap();
                let Some(forge) = r.forge.flatten() else { return; };
                if !r.mr_diffs.contains_key(&iid) && self.inflight.insert(format!("mrdiffs:{iid}")) {
                    let root2 = root.clone();
                    let tx = self.tx.clone();
                    let (epoch, generation) = (self.epoch, r.mr_gen);
                    self.spawn(move || {
                        let res = forge.get_mr(&root2, iid).map_err(|e| format!("{e:#}"));
                        let _ = tx.send((epoch, Msg::MrDetail { generation, res }));
                        Msg::MrDiffs { iid, res: forge.mr_diffs(&root2, iid).map_err(|e| format!("{e:#}")) }
                    });
                }
                if !r.mr_commits.contains_key(&iid) && self.inflight.insert(format!("mrcommits:{iid}")) {
                    self.spawn(move || Msg::MrCommits { iid, res: forge.mr_commits(&root, iid).map_err(|e| format!("{e:#}")) });
                }
            }
            Tab::Files => {
                let Some(rel) = self.sel_fs_rel() else { return };
                let r = self.repo.as_ref().unwrap();
                if !r.fs_hist.contains_key(&rel) && self.inflight.insert(format!("hist:{rel}")) {
                    self.spawn(move || Msg::FileLog { res: git::file_log(&root, &rel, 300).map_err(|e| format!("{e:#}")), path: rel });
                }
            }
        }
    }

    pub fn on_msg(&mut self, (epoch, m): (u64, Msg)) {
        // La IA va por clave y la búsqueda de proyectos no depende del repo: siempre se aplican.
        match m {
            Msg::Ai { key, res } => {
                let st = match res {
                    Ok(t) => AiState::Done(t),
                    Err(e) => AiState::Failed(e),
                };
                self.ai.insert(key, st);
                return;
            }
            Msg::AiPartial { key, text } => {
                if let Some(AiState::Running(_, partial)) = self.ai.get_mut(&key) {
                    *partial = text;
                }
                return;
            }
            Msg::Picker { base, cands } => {
                self.repo = None;
                self.epoch += 1;
                self.open_picker(base, cands);
                return;
            }
            _ => {}
        }
        if epoch != self.epoch {
            return;
        }
        let Some(r) = self.repo.as_mut() else { return };
        match m {
            Msg::Ai { .. } | Msg::AiPartial { .. } | Msg::Picker { .. } => {}
            Msg::Log { generation, skip, res } => {
                if generation != r.log_gen {
                    return;
                }
                r.log_loading = false;
                match res {
                    Ok(v) => {
                        if skip == 0 {
                            r.commits.clear();
                        }
                        if v.len() < self.cfg.page_size {
                            r.log_done = true;
                        }
                        r.commits.extend(v);
                        r.rows = graph::build_graph(&r.commits);
                        if skip == 0
                            && let Some(sha) = r.keep_sha.take()
                            && let Some(i) = r.commits.iter().position(|c| c.id == sha)
                        {
                            r.g_nav.set(i + usize::from(!r.status.is_empty()), r.commits.len() + 1);
                        }
                    }
                    Err(e) => {
                        r.log_err = Some(e);
                        r.log_done = true;
                    }
                }
                self.sel_changed.get_or_insert_with(Instant::now);
            }
            Msg::Status(v) => {
                let had_wip = !r.status.is_empty();
                r.status_map = v
                    .iter()
                    .map(|s| (s.path.clone(), if s.worktree != ' ' { s.worktree } else { s.index }))
                    .collect();
                r.status = v;
                let has_wip = !r.status.is_empty();
                // Mantener la misma fila seleccionada al aparecer/desaparecer WIP.
                if !had_wip && has_wip {
                    r.g_nav.sel += 1;
                } else if had_wip && !has_wip && r.g_nav.sel > 0 {
                    r.g_nav.sel -= 1;
                }
                let root = r.root.clone();
                let map = r.status_map.clone();
                for e in &mut r.fs_entries {
                    if e.name != ".." {
                        e.mark = mark_for(&root, &map, &e.path, e.is_dir);
                    }
                }
            }
            Msg::Files { sha, res } => {
                self.inflight.remove(&format!("files:{sha}"));
                r.files.insert(sha, res);
            }
            Msg::Detail { sha, res } => {
                self.inflight.remove(&format!("detail:{sha}"));
                if let Ok(d) = res {
                    r.details.insert(sha, d);
                }
            }
            Msg::Diff { tab, key, title, res } => {
                if self.pending_diff.as_deref() != Some(key.as_str()) {
                    return;
                }
                self.pending_diff = None;
                let view = match res {
                    Ok(t) if t.trim().is_empty() => DiffView::new(title, "(sin diferencias de texto)"),
                    Ok(t) => DiffView::new(title, &t),
                    Err(e) => DiffView::new(title, &format!("Error: {e}")),
                };
                match tab {
                    Tab::Graph => r.g_diff = Some(view),
                    Tab::Mrs => r.mr_diff = Some(view),
                    Tab::Files => r.fs_diff = Some(view),
                }
            }
            Msg::Forge(forge) => {
                r.forge = Some(forge);
                if forge.is_some() && self.tab == Tab::Mrs {
                    self.reload_mrs();
                }
            }
            Msg::Mrs { generation, page, res } => {
                if generation != r.mr_gen {
                    return;
                }
                r.mr_loading = false;
                match res {
                    Ok(v) => {
                        if v.len() < 50 {
                            r.mr_done = true;
                        }
                        if page == 1 {
                            r.mrs.clear();
                        }
                        r.mrs.extend(v);
                    }
                    Err(e) => {
                        r.mr_err = Some(e);
                        r.mr_done = true;
                    }
                }
                self.sel_changed.get_or_insert_with(Instant::now);
            }
            Msg::MrDetail { generation, res } => {
                if generation == r.mr_gen && let Ok(detail) = res
                    && let Some(mr) = r.mrs.iter_mut().find(|mr| mr.iid == detail.iid) {
                    *mr = detail;
                }
            }
            Msg::MrDiffs { iid, res } => {
                self.inflight.remove(&format!("mrdiffs:{iid}"));
                r.mr_diffs.insert(iid, res);
            }
            Msg::MrCommits { iid, res } => {
                self.inflight.remove(&format!("mrcommits:{iid}"));
                r.mr_commits.insert(iid, res.unwrap_or_default());
            }
            Msg::FileLog { path, res } => {
                self.inflight.remove(&format!("hist:{path}"));
                r.fs_hist.insert(path, res);
            }
        }
    }

    // ────────────────────────────── acciones ──────────────────────────────

    fn set_tab(&mut self, t: Tab) {
        if t == Tab::Mrs && self.repo.as_ref().and_then(|r| r.forge.flatten()).is_none() {
            return;
        }
        if self.repo.is_none() {
            return;
        }
        self.tab = t;
        self.pending_diff = None;
        self.focus = if self.diff().is_some() { Pane::Right } else { Pane::Left };
        self.sel_changed = Some(Instant::now());
        if t == Tab::Mrs {
            let r = self.repo.as_ref().unwrap();
            if r.forge.flatten().is_some() && r.mrs.is_empty() && !r.mr_loading && r.mr_err.is_none() {
                self.reload_mrs();
            }
        }
    }

    fn refresh(&mut self) {
        if let Some(r) = self.repo.as_mut() {
            r.files.clear();
            r.fs_hist.clear();
            r.branch = git::head_branch(&r.root).unwrap_or_else(|_| r.branch.clone());
        }
        self.reload_log();
        self.reload_status();
        if self.tab == Tab::Mrs {
            if let Some(r) = self.repo.as_mut() {
                r.mr_diffs.clear();
                r.mr_commits.clear();
            }
            self.reload_mrs();
        }
        self.read_dir_keep();
        self.toast("Recargado");
    }

    fn read_dir_keep(&mut self) {
        let sel = self.repo.as_ref().map_or(0, |r| r.fs_nav.sel);
        let path = self.sel_fs().map(|e| e.path.clone());
        self.read_dir();
        if let Some(r) = self.repo.as_mut() {
            let n = r.fs_entries.len();
            let i = path.and_then(|p| r.fs_entries.iter().position(|e| e.path == p)).unwrap_or(sel);
            r.fs_nav.set(i, n);
        }
    }

    /// Enter / doble clic sobre la lista enfocada.
    fn activate(&mut self) {
        if let Some(p) = self.picker.as_ref() {
            if let Some(c) = p.visible().get(p.nav.sel) {
                let path = c.path.clone();
                self.open_repo(path);
            }
            return;
        }
        let Some(r) = self.repo.as_ref() else { return };
        let root = r.root.clone();
        match (self.tab, self.focus) {
            (Tab::Graph, Pane::Left) => {
                self.focus = Pane::Right;
                if let Some(r) = self.repo.as_mut() {
                    r.g_files_nav = Nav { follow: true, ..Default::default() };
                }
            }
            (Tab::Graph, Pane::Right) => {
                if r.g_diff.is_some() {
                    return;
                }
                let Some(Ok(files)) = self.graph_files() else { return };
                let Some(f) = files.get(r.g_files_nav.sel).cloned() else { return };
                match self.graph_sel() {
                    Some(None) => {
                        let p = f.path.clone();
                        self.load_diff(Tab::Graph, format!("wip:{p}"), format!("{p}  (sin commit)"), move || working_both(&root, &p));
                    }
                    Some(Some(i)) => {
                        let sha = r.commits[i].id.clone();
                        let short = r.commits[i].short.clone();
                        let p = f.path.clone();
                        self.load_diff(Tab::Graph, format!("{sha}:{p}"), format!("{p}  @ {short}"), move || {
                            git::file_diff(&root, &sha, &p).map_err(|e| format!("{e:#}"))
                        });
                    }
                    None => {}
                }
            }
            (Tab::Mrs, Pane::Left) => {
                self.focus = Pane::Right;
                if let Some(r) = self.repo.as_mut() {
                    r.mr_files_nav = Nav { follow: true, ..Default::default() };
                }
            }
            (Tab::Mrs, Pane::Right) => {
                if r.mr_diff.is_some() {
                    return;
                }
                let Some(mr) = self.sel_mr() else { return };
                let Some(Ok(diffs)) = r.mr_diffs.get(&mr.iid) else { return };
                let Some(f) = diffs.get(r.mr_files_nav.sel) else { return };
                let title = format!("{}  ({}{})", f.new_path, r.forge.flatten().map_or("", Forge::id_prefix), mr.iid);
                let text = format!("--- a/{}\n+++ b/{}\n{}", f.old_path, f.new_path, f.diff);
                if let Some(r) = self.repo.as_mut() {
                    r.mr_diff = Some(DiffView::new(title, &text));
                }
            }
            (Tab::Files, Pane::Left) => {
                let Some(e) = self.sel_fs().cloned() else { return };
                if e.is_dir {
                    if let Some(r) = self.repo.as_mut() {
                        r.fs_dir = e.path;
                    }
                    self.read_dir();
                } else if self.sel_fs_rel().is_some() {
                    self.focus = Pane::Right;
                    if let Some(r) = self.repo.as_mut() {
                        r.fs_hist_nav = Nav { follow: true, ..Default::default() };
                    }
                }
            }
            (Tab::Files, Pane::Right) => {
                if r.fs_diff.is_some() {
                    return;
                }
                let Some(rel) = self.sel_fs_rel() else { return };
                let (wip, hist) = self.fs_hist_rows();
                let i = r.fs_hist_nav.sel;
                if wip && i == 0 {
                    self.load_diff(Tab::Files, format!("wip:{rel}"), format!("{rel}  (sin commit)"), move || working_both(&root, &rel));
                } else if let Some(Ok(h)) = hist
                    && let Some(c) = h.get(i - usize::from(wip))
                {
                    let (sha, short) = (c.id.clone(), c.short.clone());
                    self.load_diff(Tab::Files, format!("{sha}:{rel}"), format!("{rel}  @ {short}"), move || {
                        git::file_diff(&root, &sha, &rel).map_err(|e| format!("{e:#}"))
                    });
                }
            }
        }
    }

    /// Fichero anterior/siguiente de la lista del panel derecho, abriendo su diff.
    fn step_file(&mut self, d: isize) {
        let id = match self.tab {
            Tab::Graph => ListId::CommitFiles,
            Tab::Mrs => ListId::MrFiles,
            Tab::Files => ListId::FsHist,
        };
        let len = self.list_len(id);
        if let Some(n) = self.nav_mut(id) {
            n.move_by(d, len);
        }
        self.ai_nav = Nav::default();
        self.close_diff();
        self.focus = Pane::Right;
        self.activate();
    }

    fn load_diff(&mut self, tab: Tab, key: String, title: String, f: impl FnOnce() -> Result<String, String> + Send + 'static) {
        self.pending_diff = Some(key.clone());
        self.spawn(move || Msg::Diff { tab, key, title, res: f() });
    }

    fn fs_up(&mut self) {
        let Some(r) = self.repo.as_mut() else { return };
        let Some(parent) = r.fs_dir.parent().map(Path::to_path_buf) else { return };
        let came_from = r.fs_dir.clone();
        r.fs_dir = parent;
        self.read_dir();
        if let Some(r) = self.repo.as_mut()
            && let Some(i) = r.fs_entries.iter().position(|e| e.path == came_from)
        {
            r.fs_nav.set(i, r.fs_entries.len());
        }
    }

    fn open_selected_repo(&mut self) {
        if let Some(e) = self.sel_fs().cloned()
            && e.is_repo
        {
            self.open_repo(e.path);
        }
    }

    fn open_web(&mut self) {
        if let Some(url) = self.sel_mr().map(|m| m.web_url.clone()) {
            crate::platform::open_url(&url);
            self.toast(format!("Abriendo {url}"));
        }
    }

    /// Ruta real de lo seleccionado: fichero (commit, MR o explorador) o, si no hay, el proyecto.
    pub fn reveal_target(&self) -> Option<PathBuf> {
        if let Some(p) = self.picker.as_ref() {
            return p.visible().get(p.nav.sel).map(|c| c.path.clone());
        }
        let r = self.repo.as_ref()?;
        let on_right = self.focus == Pane::Right || self.diff().is_some();
        let rel: Option<String> = match self.tab {
            Tab::Graph if on_right => self
                .graph_files()
                .and_then(|f| f.ok())
                .and_then(|v| v.get(r.g_files_nav.sel).map(|f| f.path.clone())),
            Tab::Mrs if on_right => self
                .sel_mr()
                .and_then(|m| r.mr_diffs.get(&m.iid))
                .and_then(|d| d.as_ref().ok())
                .and_then(|v| v.get(r.mr_files_nav.sel).map(|f| f.new_path.clone())),
            Tab::Files => {
                return Some(self.sel_fs().filter(|e| e.name != "..").map_or(r.fs_dir.clone(), |e| e.path.clone()));
            }
            _ => None,
        };
        Some(rel.map_or(r.root.clone(), |p| r.root.join(p)))
    }

    /// Abre el gestor de archivos en la ubicación real, con el elemento seleccionado.
    fn reveal(&mut self) {
        let Some(mut path) = self.reveal_target() else { return };
        // Fichero borrado o aún no presente: su carpeta existente más cercana.
        while !path.exists() {
            match path.parent() {
                Some(p) => path = p.to_path_buf(),
                None => return,
            }
        }
        self.toast(format!("Abriendo {}", path.display()));
        std::thread::spawn(move || reveal_path(&path));
    }

    fn copy_sha(&mut self) {
        let txt = match self.tab {
            Tab::Mrs => self.sel_mr().map(|m| m.web_url.clone()),
            _ => self.sel_commit().map(|c| c.id.clone()),
        };
        if let Some(t) = txt {
            use std::io::Write;
            let mut out = std::io::stdout();
            let _ = write!(out, "\x1b]52;c;{}\x07", base64(t.as_bytes()));
            let _ = out.flush();
            self.toast(format!("Copiado: {t}"));
        }
    }

    fn set_mr_state(&mut self, st: MrState) {
        if let Some(r) = self.repo.as_mut()
            && r.mr_state != st
        {
            r.mr_state = st;
            self.reload_mrs();
        }
    }

    pub fn press(&mut self, b: Btn) {
        match b {
            Btn::Explain => self.explain(false),
            Btn::CloseDiff => {
                self.close_diff();
            }
            Btn::MrState(s) => self.set_mr_state(s),
            Btn::OpenRepo => self.open_selected_repo(),
            Btn::Quit => self.quit = true,
            Btn::Help => self.help = !self.help,
            Btn::AiMax => self.ai_max = !self.ai_max,
            Btn::Reveal => self.reveal(),
            Btn::Refresh => self.refresh(),
            Btn::OpenWeb => self.open_web(),
            Btn::Back => {
                let Some(r) = self.repo.as_ref() else { return };
                let base = r.root.parent().map(Path::to_path_buf).unwrap_or_default();
                let cfg = self.cfg.clone();
                self.toast("Buscando proyectos…");
                self.spawn(move || {
                    let ctx = crate::discover::HerdrCtx::from_env();
                    let cands = crate::discover::rank(&base, &crate::discover::find_repos(&base, cfg.scan_depth), &cfg, &ctx);
                    Msg::Picker { base, cands }
                });
            }
        }
    }

    // ───────────────────────────── navegación ─────────────────────────────

    fn focused_list(&self) -> ListId {
        if self.picker.is_some() {
            return ListId::Picker;
        }
        if self.diff().is_some() && self.focus == Pane::Right {
            return ListId::Diff;
        }
        match (self.tab, self.focus) {
            (Tab::Graph, Pane::Left) => ListId::Commits,
            (Tab::Graph, Pane::Right) => ListId::CommitFiles,
            (Tab::Mrs, Pane::Left) => ListId::Mrs,
            (Tab::Mrs, Pane::Right) => ListId::MrFiles,
            (Tab::Files, Pane::Left) => ListId::Fs,
            (Tab::Files, Pane::Right) => ListId::FsHist,
        }
    }

    fn list_len(&self, id: ListId) -> usize {
        match id {
            ListId::Picker => self.picker.as_ref().map_or(0, |p| p.visible().len()),
            ListId::Commits => self.graph_len(),
            ListId::CommitFiles => self.graph_files().and_then(|r| r.ok()).map_or(0, |v| v.len()),
            ListId::Diff => self.diff().map_or(0, |d| d.lines.len()),
            ListId::Mrs => self.repo.as_ref().map_or(0, |r| r.mrs.len()),
            ListId::MrFiles => self
                .sel_mr()
                .and_then(|m| self.repo.as_ref()?.mr_diffs.get(&m.iid))
                .and_then(|r| r.as_ref().ok())
                .map_or(0, |v| v.len()),
            ListId::Fs => self.repo.as_ref().map_or(0, |r| r.fs_entries.len()),
            ListId::FsHist => self.fs_hist_len(),
            ListId::Ai => usize::MAX / 2,
        }
    }

    fn nav_mut(&mut self, id: ListId) -> Option<&mut Nav> {
        if id == ListId::Picker {
            return self.picker.as_mut().map(|p| &mut p.nav);
        }
        if id == ListId::Ai {
            return Some(&mut self.ai_nav);
        }
        let r = self.repo.as_mut()?;
        Some(match id {
            ListId::Commits => &mut r.g_nav,
            ListId::CommitFiles => &mut r.g_files_nav,
            ListId::Mrs => &mut r.mr_nav,
            ListId::MrFiles => &mut r.mr_files_nav,
            ListId::Fs => &mut r.fs_nav,
            ListId::FsHist => &mut r.fs_hist_nav,
            ListId::Diff | ListId::Picker | ListId::Ai => return None,
        })
    }

    fn move_sel(&mut self, id: ListId, d: isize) {
        let len = self.list_len(id);
        if id == ListId::Diff {
            self.scroll_diff(d);
            return;
        }
        if let Some(n) = self.nav_mut(id) {
            let before = n.sel;
            n.move_by(d, len);
            if n.sel != before {
                self.on_sel_change(id);
            }
        }
    }

    /// Desplaza la explicación (el render limita al final del texto).
    fn scroll_ai(&mut self, d: isize) {
        self.ai_nav.off = (self.ai_nav.off as isize + d).max(0) as usize;
    }

    fn scroll_diff(&mut self, d: isize) {
        let h = self.diff_h;
        if let Some(dv) = self.diff_mut() {
            let max = dv.lines.len().saturating_sub(h.max(1));
            dv.scroll = (dv.scroll as isize + d).clamp(0, max as isize) as usize;
        }
    }

    fn on_sel_change(&mut self, id: ListId) {
        self.sel_changed = Some(Instant::now());
        self.pending_diff = None;
        self.ai_nav = Nav::default();
        if let Some(r) = self.repo.as_mut() {
            match id {
                ListId::Commits => r.g_files_nav = Nav { follow: true, ..Default::default() },
                ListId::Mrs => r.mr_files_nav = Nav { follow: true, ..Default::default() },
                ListId::Fs => r.fs_hist_nav = Nav { follow: true, ..Default::default() },
                _ => {}
            }
        }
    }

    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::Mouse(m) => self.on_mouse(m),
            _ => {}
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('q')) {
            self.quit = true;
            return;
        }
        if self.help {
            self.help = false;
            return;
        }
        // Selector: escribir filtra (Ctrl+O muestra la carpeta del proyecto).
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('o') {
            self.reveal();
            return;
        }
        if let Some(p) = self.picker.as_mut() {
            match k.code {
                KeyCode::Esc if !p.filter.is_empty() => p.filter.clear(),
                KeyCode::Esc => self.quit = true,
                KeyCode::Enter => self.activate(),
                KeyCode::Up => self.move_sel(ListId::Picker, -1),
                KeyCode::Down => self.move_sel(ListId::Picker, 1),
                KeyCode::PageUp => self.move_sel(ListId::Picker, -10),
                KeyCode::PageDown => self.move_sel(ListId::Picker, 10),
                KeyCode::Backspace => {
                    p.filter.pop();
                    p.nav.set(0, 1);
                }
                KeyCode::Char(c) => {
                    p.filter.push(c);
                    p.nav.set(0, 1);
                }
                _ => {}
            }
            return;
        }
        let id = self.focused_list();
        let page = match id {
            ListId::Diff => self.diff_h.max(2) as isize - 1,
            _ => self.nav_mut(id).map_or(10, |n| n.page()),
        };
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => {
                if self.close_diff() {
                    self.focus = Pane::Right;
                } else if self.focus == Pane::Right {
                    self.focus = Pane::Left;
                } else {
                    self.quit = true;
                }
            }
            KeyCode::Char('?') => self.help = true,
            KeyCode::Char('1') => self.set_tab(Tab::Graph),
            KeyCode::Char('2') => self.set_tab(Tab::Mrs),
            KeyCode::Char('3') => self.set_tab(Tab::Files),
            KeyCode::Tab => {
                self.focus = if self.focus == Pane::Left { Pane::Right } else { Pane::Left };
            }
            KeyCode::BackTab => self.set_tab(match self.tab {
                Tab::Graph => Tab::Files,
                Tab::Mrs => Tab::Graph,
                Tab::Files if self.repo.as_ref().and_then(|r| r.forge.flatten()).is_some() => Tab::Mrs,
                Tab::Files => Tab::Graph,
            }),
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(id, -1),
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(id, 1),
            KeyCode::PageUp | KeyCode::Char('u') if k.code == KeyCode::PageUp || k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_sel(id, -page)
            }
            KeyCode::PageDown | KeyCode::Char('d') if k.code == KeyCode::PageDown || k.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_sel(id, page)
            }
            KeyCode::Home | KeyCode::Char('g') => self.move_sel(id, -(1 << 30)),
            KeyCode::End | KeyCode::Char('G') => self.move_sel(id, 1 << 30),
            KeyCode::Enter | KeyCode::Char('l') => self.activate(),
            KeyCode::Right
                if self.focus == Pane::Left => {
                    self.activate();
                }
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace => {
                if self.close_diff() {
                    self.focus = Pane::Right;
                } else if self.focus == Pane::Right {
                    self.focus = Pane::Left;
                } else if self.tab == Tab::Files {
                    self.fs_up();
                }
            }
            KeyCode::Char('e') => self.explain(false),
            KeyCode::Char('E') => self.explain(true),
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('o') => match self.tab {
                Tab::Mrs => self.open_web(),
                Tab::Files => self.open_selected_repo(),
                Tab::Graph => {}
            },
            KeyCode::Char('y') => self.copy_sha(),
            KeyCode::Char('O') => self.reveal(),
            KeyCode::Char('p') => self.press(Btn::Back),
            KeyCode::Char('n') | KeyCode::Char('N') if id == ListId::Diff => {
                let fwd = k.code == KeyCode::Char('n');
                if let Some(dv) = self.diff_mut() {
                    let cur = dv.scroll;
                    let target = if fwd {
                        dv.hunks().find(|&i| i > cur)
                    } else {
                        dv.hunks().filter(|&i| i < cur).last()
                    };
                    if let Some(t) = target {
                        dv.scroll = t;
                    }
                }
            }
            KeyCode::Char('a') if self.tab == Tab::Mrs => {
                let next = match self.repo.as_ref().map(|r| r.mr_state) {
                    Some(MrState::Opened) => MrState::Merged,
                    Some(MrState::Merged) => MrState::Closed,
                    Some(MrState::Closed) => MrState::All,
                    _ => MrState::Opened,
                };
                self.set_mr_state(next);
            }
            KeyCode::Char('[') => self.step_file(-1),
            KeyCode::Char(']') => self.step_file(1),
            KeyCode::Char('J') => self.scroll_ai(3),
            KeyCode::Char('K') => self.scroll_ai(-3),
            KeyCode::Char('z') => self.ai_max = !self.ai_max,
            _ => {}
        }
    }

    fn hit_at(&self, x: u16, y: u16) -> Option<(Rect, Hit)> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .copied()
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        match m.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let d: isize = if m.kind == MouseEventKind::ScrollUp { -3 } else { 3 };
                if let Some((_, Hit::List(id))) = self.hit_at(x, y) {
                    if id == ListId::Diff {
                        self.scroll_diff(d);
                    } else if id == ListId::Ai {
                        self.scroll_ai(d);
                    } else {
                        let len = self.list_len(id);
                        if let Some(n) = self.nav_mut(id) {
                            n.scroll(d, len);
                        }
                        // Paginación al hacer scroll hasta el final.
                        if id == ListId::Commits || id == ListId::Mrs {
                            self.sel_changed.get_or_insert_with(Instant::now);
                            let near_end = self.nav_mut(id).is_some_and(|n| n.off + n.h * 2 >= len);
                            if near_end {
                                if id == ListId::Commits { self.more_log() } else { self.more_mrs() }
                            }
                        }
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let dbl = self
                    .last_click
                    .is_some_and(|(t, lx, ly)| t.elapsed() < Duration::from_millis(400) && lx == x && ly == y);
                self.last_click = Some((Instant::now(), x, y));
                if self.help {
                    self.help = false;
                    return;
                }
                let Some((rect, hit)) = self.hit_at(x, y) else { return };
                match hit {
                    Hit::Tab(t) => self.set_tab(t),
                    Hit::Btn(b) => self.press(b),
                    Hit::List(id) => {
                        self.focus = match id {
                            ListId::Commits | ListId::Mrs | ListId::Fs | ListId::Picker => Pane::Left,
                            _ => Pane::Right,
                        };
                        if matches!(id, ListId::Diff | ListId::Ai) {
                            return;
                        }
                        let len = self.list_len(id);
                        let Some(n) = self.nav_mut(id) else { return };
                        let row = n.off + (y - rect.y) as usize;
                        if row >= len {
                            return;
                        }
                        let changed = n.sel != row;
                        n.sel = row;
                        n.follow = true;
                        if changed {
                            self.on_sel_change(id);
                        }
                        // Clic en fichero → abre diff directamente; doble clic en el resto → activar.
                        let single_opens = matches!(id, ListId::CommitFiles | ListId::MrFiles | ListId::FsHist | ListId::Picker);
                        if single_opens || dbl {
                            if matches!(id, ListId::CommitFiles | ListId::MrFiles | ListId::FsHist) {
                                self.close_diff();
                            }
                            self.activate();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Muestra el fichero o directorio en el gestor de ficheros del sistema (ver `platform::reveal`).
fn reveal_path(path: &Path) {
    crate::platform::reveal(path);
}

/// Codifica una ruta para una URI file:// (respeta '/'). La usa `platform::reveal` en Linux.
#[cfg_attr(windows, allow(dead_code))] // en Windows `reveal` usa explorer y nadie la llama
pub(crate) fn uri_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn mark_for(root: &Path, map: &HashMap<String, char>, path: &Path, is_dir: bool) -> Option<char> {
    let rel = path.strip_prefix(root).ok()?.to_string_lossy().into_owned();
    if is_dir {
        let pre = format!("{rel}/");
        map.keys().any(|k| k.starts_with(&pre)).then_some('•')
    } else {
        map.get(&rel).copied()
    }
}

fn working_both(root: &Path, p: &str) -> Result<String, String> {
    let staged = git::working_diff(root, p, true).unwrap_or_default();
    let unstaged = git::working_diff(root, p, false).map_err(|e| format!("{e:#}"))?;
    Ok(match (staged.trim().is_empty(), unstaged.trim().is_empty()) {
        (false, false) => format!("{staged}\n{unstaged}"),
        (false, true) => staged,
        _ => unstaged,
    })
}

fn git_out(root: &Path, args: &[&str]) -> Result<String, String> {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for ch in b.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if ch.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if ch.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

#[cfg(test)]
pub(crate) fn test_repo(forge: Option<Forge>) -> Repo {
    Repo {
        root: PathBuf::from("test-repo"), name: "test-repo".into(), branch: "main".into(),
        remote: Some("https://github.com/owner/repo.git".into()),
        commits: vec![], rows: vec![], log_gen: 0, log_loading: false, log_done: true,
        log_err: None, status: vec![], status_map: HashMap::new(),
        g_nav: Nav::default(), g_files_nav: Nav::default(), files: HashMap::new(),
        details: HashMap::new(), g_diff: None, keep_sha: None,
        forge: Some(forge), mrs: vec![MergeRequest {
            iid: 12, title: "Example change".into(), state: "opened".into(),
            sha: Some("abc123".into()), ..Default::default()
        }], mr_state: MrState::All, mr_page: 1, mr_loading: false,
        mr_done: true, mr_err: None, mr_gen: 0,
        mr_nav: Nav::default(), mr_files_nav: Nav::default(), mr_diffs: HashMap::new(),
        mr_commits: HashMap::new(), mr_diff: None,
        fs_dir: PathBuf::from("test-repo"), fs_entries: vec![],
        fs_nav: Nav::default(), fs_hist_nav: Nav::default(), fs_hist: HashMap::new(), fs_diff: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_codifica_espacios_y_acentos() {
        assert_eq!(uri_encode("/home/a b/año.txt"), "/home/a%20b/a%C3%B1o.txt");
    }

    #[test]
    fn b64() {
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"ab"), "YWI=");
    }

    #[test]
    fn nav_fit_follows() {
        let mut n = Nav { follow: true, ..Default::default() };
        n.move_by(30, 100);
        n.fit(10, 100);
        assert_eq!((n.sel, n.off), (30, 21));
        n.scroll(-5, 100);
        n.fit(10, 100);
        assert_eq!(n.off, 16);
    }
    #[test]
    fn ai_request_uses_forge_terms_and_project_cache_identity() {
        for (forge, id) in [(Forge::GitHub, "PR #12"), (Forge::GitLab, "MR !12")] {
            let mut app = App::new(Config::default());
            app.repo = Some(test_repo(Some(forge)));
            app.tab = Tab::Mrs;
            let job = app.ai_job().unwrap();
            assert_eq!(job.label, id);
            assert!(job.context.starts_with(id));
            assert_eq!(job.key, "mr:https://github.com/owner/repo.git:12:abc123");
            app.repo.as_mut().unwrap().mr_diffs.insert(12, Ok(vec![MrFileDiff {
                new_path: "x.rs".into(), ..Default::default()
            }]));
            app.focus = Pane::Right;
            let job = app.ai_job().unwrap();
            assert!(job.context.starts_with(id));
            assert!(job.label.contains(&id[3..]));
        }
    }

    #[test]
    fn unavailable_forge_cannot_be_selected_and_is_skipped_by_backtab() {
        let mut app = App::new(Config::default());
        app.repo = Some(test_repo(None));
        app.set_tab(Tab::Mrs);
        assert_eq!(app.tab, Tab::Graph);
        app.tab = Tab::Files;
        app.on_event(Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)));
        assert_eq!(app.tab, Tab::Graph);
    }

}
