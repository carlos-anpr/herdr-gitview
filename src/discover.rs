//! Detección del proyecto a abrir: repo actual, o candidatos ordenados por actividad de agentes.

use crate::config::Config;
use crate::model::Candidate;
use crate::platform;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "vendor", "coverage", ".next", "__pycache__", "venv",
];
const TAIL_BYTES: u64 = 768 * 1024;
const ACTIVE_SCORE: f64 = 20.0;

pub enum Discovery {
    /// Abrir directamente este repo.
    Direct(PathBuf),
    /// Mostrar selector (activos primero). Puede venir vacío.
    Pick(Vec<Candidate>),
}

/// Contexto de herdr leído del entorno (popup de keybinding o plugin).
#[derive(Debug, Clone, Default)]
pub struct HerdrCtx {
    pub workspace_id: Option<String>,
    pub pane_cwd: Option<PathBuf>,
}

impl HerdrCtx {
    pub fn from_env() -> Self {
        let mut ctx = HerdrCtx {
            workspace_id: env_nonempty("HERDR_ACTIVE_WORKSPACE_ID")
                .or_else(|| env_nonempty("HERDR_WORKSPACE_ID")),
            pane_cwd: env_nonempty("HERDR_ACTIVE_PANE_CWD").map(PathBuf::from),
        };
        if let Some(raw) = env_nonempty("HERDR_PLUGIN_CONTEXT_JSON")
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw)
        {
            if ctx.workspace_id.is_none() {
                ctx.workspace_id = v["workspace_id"].as_str().map(String::from);
            }
            if ctx.pane_cwd.is_none() {
                ctx.pane_cwd = v["focused_pane_cwd"]
                    .as_str()
                    .or_else(|| v["workspace_cwd"].as_str())
                    .map(PathBuf::from);
            }
        }
        ctx
    }
}

fn env_nonempty(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|s| !s.is_empty())
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn mtime(p: &Path) -> Option<i64> {
    fs::metadata(p)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// `.git` como directorio o fichero (worktrees/submódulos).
fn is_repo_dir(p: &Path) -> bool {
    p.join(".git").exists()
}

/// Sube desde `p` hasta encontrar un repo.
fn enclosing_repo(p: &Path) -> Option<PathBuf> {
    let mut cur = Some(p);
    while let Some(c) = cur {
        if is_repo_dir(c) {
            return Some(c.to_path_buf());
        }
        cur = c.parent();
    }
    None
}

/// Repos bajo `base` hasta `depth` niveles; no entra dentro de un repo.
pub fn find_repos(base: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(base, depth, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
            continue;
        }
        let p = e.path();
        if is_repo_dir(&p) {
            out.push(p);
        } else if depth > 1 {
            walk(&p, depth - 1, out);
        }
    }
}

/// Resuelve qué abrir partiendo de `start`.
pub fn discover(start: &Path, cfg: &Config, ctx: &HerdrCtx) -> Discovery {
    if let Some(repo) = enclosing_repo(start) {
        return Discovery::Direct(repo);
    }
    let repos = find_repos(start, cfg.scan_depth.max(1));
    if repos.len() == 1 {
        return Discovery::Direct(repos[0].clone());
    }
    let cands = rank(start, &repos, cfg, ctx);
    let active: Vec<&Candidate> = cands.iter().filter(|c| c.active).collect();
    if active.len() == 1 {
        return Discovery::Direct(active[0].path.clone());
    }
    Discovery::Pick(cands)
}

/// Puntúa cada repo según señales de actividad reciente.
pub fn rank(start: &Path, repos: &[PathBuf], cfg: &Config, ctx: &HerdrCtx) -> Vec<Candidate> {
    let since = now() - (cfg.discover_hours as i64) * 3600;
    let mut cands: Vec<Candidate> = repos
        .iter()
        .map(|p| Candidate {
            path: p.clone(),
            name: p
                .strip_prefix(start)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string_lossy().into_owned()),
            ..Default::default()
        })
        .collect();

    // Señales en paralelo: paneles herdr, transcripciones, git.
    let panes = std::thread::spawn(herdr_panes);
    let repos_owned: Vec<PathBuf> = repos.to_vec();
    let start_owned = start.to_path_buf();
    let hits = std::thread::spawn(move || transcript_hits(&start_owned, &repos_owned, since));
    let git_info: Vec<(Option<String>, usize, Option<i64>)> = par_map(repos, |r| git_signals(r), (None, 0, None));

    for (c, (branch, dirty, head_t)) in cands.iter_mut().zip(git_info) {
        c.branch = branch;
        c.dirty = dirty;
        if let Some(t) = head_t {
            c.last_activity = Some(t);
            if t >= since {
                c.score += 15.0;
                c.reasons.push(format!("git activo {}", ago(t)));
            }
        }
        if dirty > 0 {
            c.score += 3.0;
            c.reasons.push(format!("{dirty} cambios sin commit"));
        }
    }

    let mut seen_pane: Vec<bool> = vec![false; cands.len()];
    for p in panes.join().unwrap_or_default() {
        let Some(i) = owner(&cands, &p.cwd) else { continue };
        let c = &mut cands[i];
        let same_ws = ctx.workspace_id.as_deref() == Some(p.workspace.as_str());
        // El primer agente cuenta entero; los siguientes apenas suman.
        let first = !std::mem::replace(&mut seen_pane[i], true);
        c.score += match (same_ws, first) {
            (true, _) => 50.0,
            (false, true) => 20.0,
            (false, false) => 3.0,
        };
        if p.working {
            c.score += 10.0;
        }
        c.last_activity = Some(now());
        c.reasons.push(format!(
            "{} en {}{}",
            p.agent,
            p.pane,
            if p.working { " (trabajando)" } else { "" }
        ));
    }

    for (i, (n, t)) in hits.join().unwrap_or_default() {
        let c = &mut cands[i];
        c.score += 10.0 * (n.min(5) as f64);
        c.last_activity = Some(c.last_activity.unwrap_or(0).max(t));
        c.reasons.push(format!("sesión IA {} ({n} refs)", ago(t)));
    }

    for c in &mut cands {
        c.active = c.score >= ACTIVE_SCORE;
    }
    cands.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.score.total_cmp(&a.score))
            .then(b.last_activity.cmp(&a.last_activity))
            .then(a.name.cmp(&b.name))
    });
    cands
}

/// `map` en paralelo con como mucho `MAX_THREADS` hilos; conserva el orden.
const MAX_THREADS: usize = 16;

fn par_map<T: Sync, R: Send + Clone>(items: &[T], f: impl Fn(&T) -> R + Sync, fallback: R) -> Vec<R> {
    if items.is_empty() {
        return Vec::new();
    }
    let chunk = items.len().div_ceil(MAX_THREADS);
    std::thread::scope(|s| {
        let f = &f;
        let hs: Vec<_> = items.chunks(chunk).map(|c| s.spawn(move || c.iter().map(f).collect::<Vec<R>>())).collect();
        hs.into_iter()
            .zip(items.chunks(chunk))
            .flat_map(|(h, c)| h.join().unwrap_or_else(|_| vec![fallback.clone(); c.len()]))
            .collect()
    })
}

/// Índice del candidato que contiene `p` (el más profundo).
fn owner(cands: &[Candidate], p: &Path) -> Option<usize> {
    let p_norm = norm_path(&p.to_string_lossy());
    cands
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let repo = norm_path(&c.path.to_string_lossy());
            p_norm == repo
                || (p_norm.starts_with(&repo) && matches!(p_norm.as_bytes().get(repo.len()), Some(b'\\')))
        })
        .max_by_key(|(_, c)| c.path.as_os_str().len())
        .map(|(i, _)| i)
}

pub fn ago(t: i64) -> String {
    let d = (now() - t).max(0);
    match d {
        0..=59 => "hace un momento".into(),
        60..=3599 => format!("hace {} min", d / 60),
        3600..=86399 => format!("hace {} h", d / 3600),
        _ => format!("hace {} d", d / 86400),
    }
}

fn git_signals(repo: &Path) -> (Option<String>, usize, Option<i64>) {
    let gitdir = git_dir(repo);
    let branch = gitdir.as_ref().and_then(|g| {
        let head = fs::read_to_string(g.join("HEAD")).ok()?;
        Some(match head.trim().strip_prefix("ref: refs/heads/") {
            Some(b) => b.to_string(),
            None => head.trim().chars().take(8).collect(),
        })
    });
    let head_t = gitdir.as_ref().and_then(|g| mtime(&g.join("logs/HEAD")));
    let dirty = platform::cmd("git")
        .args(["-C"])
        .arg(repo)
        .args(["status", "--porcelain=v1", "-uno"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| o.stdout.split(|b| *b == b'\n').filter(|l| !l.is_empty()).count())
        .unwrap_or(0);
    (branch, dirty, head_t)
}

/// Resuelve `.git` (dir o fichero `gitdir: ...`).
fn git_dir(repo: &Path) -> Option<PathBuf> {
    let g = repo.join(".git");
    if g.is_dir() {
        return Some(g);
    }
    let s = fs::read_to_string(&g).ok()?;
    let p = PathBuf::from(s.trim().strip_prefix("gitdir:")?.trim());
    Some(if p.is_absolute() { p } else { repo.join(p) })
}

struct PaneInfo {
    pane: String,
    workspace: String,
    agent: String,
    cwd: PathBuf,
    working: bool,
}

fn herdr_panes() -> Vec<PaneInfo> {
    let bin = env_nonempty("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let Ok(out) = platform::cmd(bin).args(["pane", "list"]).output() else { return vec![] };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return vec![] };
    let self_pane = env_nonempty("HERDR_PANE_ID");
    v["result"]["panes"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let agent = p["agent"].as_str()?;
                    let pane = p["pane_id"].as_str()?.to_string();
                    if self_pane.as_deref() == Some(pane.as_str()) {
                        return None;
                    }
                    let cwd = p["foreground_cwd"].as_str().or_else(|| p["cwd"].as_str())?;
                    Some(PaneInfo {
                        pane,
                        workspace: p["workspace_id"].as_str().unwrap_or("").to_string(),
                        agent: agent.to_string(),
                        cwd: PathBuf::from(cwd),
                        working: p["agent_status"].as_str() == Some("working"),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Claude Code codifica la ruta del proyecto sustituyendo todo lo no alfanumérico por '-'.
/// En Windows `C:\Users\UR\Documents` → `C--Users-UR-Documents`.
fn claude_encode(p: &Path) -> String {
    p.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Normaliza una ruta para compararla: separadores unificados a `\`, sin duplicados (las
/// transcripciones JSON escapan `\\`) y, en Windows, sin distinguir mayúsculas.
fn norm_path(s: &str) -> String {
    let s: Cow<'_, str> = if cfg!(windows) { Cow::Owned(s.to_lowercase()) } else { Cow::Borrowed(s) };
    let mut out = String::with_capacity(s.len() + 1);
    let mut after_sep = false;
    for c in s.chars() {
        if c == '/' || c == '\\' {
            if !after_sep {
                out.push('\\');
            }
            after_sep = true;
        } else {
            out.push(c);
            after_sep = false;
        }
    }
    out
}

/// `hay` empieza por `prefix` (sin distinguir mayúsculas en Windows).
fn starts_with_ci(hay: &str, prefix: &str) -> bool {
    match hay.get(..prefix.len()) {
        Some(head) if cfg!(windows) => head.eq_ignore_ascii_case(prefix),
        Some(head) => head == prefix,
        None => false,
    }
}

/// Texto donde buscar menciones (minúsculas en Windows, para comparar rutas sin caja).
fn lower_text(s: &str) -> Cow<'_, str> {
    if cfg!(windows) { Cow::Owned(s.to_lowercase()) } else { Cow::Borrowed(s) }
}

/// Variantes con las que una ruta puede aparecer en una transcripción: tal cual, con las
/// barras escapadas de JSON (`\\`) y con `/`.
fn path_needles(needle: &str) -> Vec<String> {
    let mut out = vec![needle.to_string()];
    #[cfg(windows)]
    for alt in [needle.replace('\\', "\\\\"), needle.replace('\\', "/")] {
        if !out.contains(&alt) {
            out.push(alt);
        }
    }
    out
}

/// Carácter que cierra una mención de ruta (en Windows `\` es separador, no final).
fn is_mention_end(c: char) -> bool {
    c == '"' || c == '\'' || c == '`' || c.is_whitespace() || c == ')' || (!cfg!(windows) && c == '\\')
}

/// Ficheros de sesión de agentes modificados desde `since` (Claude Code + Codex).
fn session_files(start: &Path, since: i64) -> Vec<(PathBuf, i64)> {
    let mut files = Vec::new();
    let Some(home) = platform::home_dir() else { return files };
    let prefix = claude_encode(start);
    if let Ok(rd) = fs::read_dir(home.join(".claude/projects")) {
        for d in rd.flatten() {
            if !starts_with_ci(&d.file_name().to_string_lossy(), &prefix) {
                continue;
            }
            collect_recent(&d.path(), since, &mut files);
        }
    }
    // Codex: ~/.codex/sessions/YYYY/MM/DD/*.jsonl — hoy y los días que cubra la ventana.
    let days = ((now() - since) / 86400 + 1) as u64;
    for back in 0..=days {
        let t = SystemTime::now() - Duration::from_secs(back * 86400);
        let (y, m, d) = ymd(t);
        let dir = home.join(format!(".codex/sessions/{y:04}/{m:02}/{d:02}"));
        collect_recent(&dir, since, &mut files);
    }
    files
}

fn collect_recent(dir: &Path, since: i64, out: &mut Vec<(PathBuf, i64)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for f in rd.flatten() {
        let p = f.path();
        if p.extension().is_some_and(|e| e == "jsonl")
            && let Some(t) = mtime(&p)
            && t >= since
        {
            out.push((p, t));
        }
    }
}

/// Para cada repo: (nº de ficheros de sesión que lo mencionan, mtime más reciente).
fn transcript_hits(start: &Path, repos: &[PathBuf], since: i64) -> HashMap<usize, (usize, i64)> {
    let files = session_files(start, since);
    let needle = start.to_string_lossy().into_owned();
    let mut res: HashMap<usize, (usize, i64)> = HashMap::new();
    let per_file: Vec<(Vec<usize>, i64)> =
        par_map(&files, |(p, t)| (repos_in_file(p, &needle, repos), *t), (Vec::new(), 0));
    for (idxs, t) in per_file {
        for i in idxs {
            let e = res.entry(i).or_insert((0, 0));
            e.0 += 1;
            e.1 = e.1.max(t);
        }
    }
    res
}

/// Repos mencionados (por ruta absoluta) en la cola de un fichero de sesión.
fn repos_in_file(path: &Path, needle: &str, repos: &[PathBuf]) -> Vec<usize> {
    if needle.is_empty() || repos.is_empty() {
        return vec![];
    }
    let Ok(mut f) = fs::File::open(path) else { return vec![] };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > TAIL_BYTES {
        let _ = f.seek(SeekFrom::Start(len - TAIL_BYTES));
    }
    let mut buf = Vec::with_capacity(TAIL_BYTES.min(len) as usize);
    if f.read_to_end(&mut buf).is_err() {
        return vec![];
    }
    let text = String::from_utf8_lossy(&buf);
    let hay = lower_text(&text);
    let needles = path_needles(&lower_text(needle));
    let repo_strs: Vec<String> = repos.iter().map(|r| norm_path(&r.to_string_lossy())).collect();
    let mut found = vec![false; repos.len()];
    for needle in &needles {
        let mut from = 0;
        while let Some(hit) = hay[from..].find(needle.as_str()) {
            let pos = from + hit;
            from = pos + needle.len();
            if !is_tool_path(&hay[..pos]) {
                continue;
            }
            let rest = &hay[pos..];
            let end = rest.find(is_mention_end).unwrap_or(rest.len());
            let mentioned = norm_path(&rest[..end]);
            let best = repo_strs
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    mentioned.starts_with(r.as_str())
                        && matches!(mentioned.as_bytes().get(r.len()), None | Some(b'\\'))
                })
                .max_by_key(|(_, r)| r.len());
            if let Some((i, _)) = best {
                found[i] = true;
            }
        }
    }
    found.iter().enumerate().filter(|(_, f)| **f).map(|(i, _)| i).collect()
}

/// La mención es una ruta usada por una herramienta del agente (no texto de una salida).
fn is_tool_path(before: &str) -> bool {
    let mut start = before.len().saturating_sub(24);
    while !before.is_char_boundary(start) {
        start += 1;
    }
    let tail = &before[start..];
    const KEYS: [&str; 9] = [
        "file_path\":\"", "\"cwd\":\"", "\"path\":\"", "notebook_path\":\"", "\"workdir\":\"",
        "cd ", "-C ", "--cwd ", "\"cwd\": \"",
    ];
    KEYS.iter().any(|k| tail.ends_with(k))
}

/// Fecha civil UTC (algoritmo de Howard Hinnant).
fn ymd(t: SystemTime) -> (i64, u32, u32) {
    let days = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) / 86400;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_like_claude() {
        assert_eq!(
            claude_encode(Path::new("/home/x/Proyectos/a.b_c")),
            "-home-x-Proyectos-a-b-c"
        );
    }

    /// En Windows la ruta de `herdr pane list` llega como `C:\Users\...`.
    #[cfg(windows)]
    #[test]
    fn encode_like_claude_windows() {
        assert_eq!(
            claude_encode(Path::new(r"C:\Users\UR\Documents")),
            "C--Users-UR-Documents"
        );
        assert_eq!(
            claude_encode(Path::new(r"C:\Users\UR\Documents\herdr-gitview")),
            "C--Users-UR-Documents-herdr-gitview"
        );
    }

    /// Rutas con barras escapadas (JSON), con `/` y con otra caja.
    #[cfg(windows)]
    #[test]
    fn mentions_con_barras_escapadas_y_caja() {
        let dir = std::env::temp_dir().join(format!("gv-disc-win-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.jsonl");
        fs::write(
            &f,
            r#"{"file_path":"C:\\Users\\UR\\Documents\\repo\\src\\a.rs"} {"cwd":"C:/Users/UR/Documents/OTRO"} cd C:\Users\UR\Documents\suelto"#,
        )
        .unwrap();
        let repos = vec![
            PathBuf::from(r"C:\Users\UR\Documents\repo"),
            PathBuf::from(r"C:\Users\UR\Documents\otro"),
        ];
        let got = repos_in_file(&f, r"C:\Users\UR\Documents", &repos);
        assert_eq!(got, vec![0, 1]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ymd_known_date() {
        let t = UNIX_EPOCH + Duration::from_secs(1_760_313_600); // 2025-10-13
        assert_eq!(ymd(t), (2025, 10, 13));
    }

    #[test]
    fn mentions_pick_deepest_repo() {
        let dir = std::env::temp_dir().join(format!("gv-disc-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.jsonl");
        fs::write(
            &f,
            r#"{"file_path":"/p/Proyectos/dir/back/src/a.rs"} {"cwd":"/p/Proyectos/other"} cd /p/Proyectos/dirx salida /p/Proyectos/dir/front"#,
        )
        .unwrap();
        let repos = vec![
            PathBuf::from("/p/Proyectos/dir/back"),
            PathBuf::from("/p/Proyectos/dir/front"),
            PathBuf::from("/p/Proyectos/other"),
            PathBuf::from("/p/Proyectos/dir"),
        ];
        let got = repos_in_file(&f, "/p/Proyectos/", &repos);
        assert_eq!(got, vec![0, 2]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_nested_repos_and_skips_node_modules() {
        let base = std::env::temp_dir().join(format!("gv-repos-{}", std::process::id()));
        for d in ["a/.git", "b/c/.git", "node_modules/x/.git", "a/sub/.git"] {
            fs::create_dir_all(base.join(d)).unwrap();
        }
        let got: Vec<String> = find_repos(&base, 3)
            .iter()
            .map(|p| p.strip_prefix(&base).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(got, vec!["a", "b/c"]);
        fs::remove_dir_all(&base).ok();
    }
}
