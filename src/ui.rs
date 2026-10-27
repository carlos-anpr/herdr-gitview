//! Render con ratatui. Cada frame reconstruye `app.hits` para el ratón.

use crate::app::*;
use crate::diff::Kind;
use crate::model::*;
use crate::text::*;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const MERGED: Color = Color::Rgb(0xa3, 0x71, 0xf7);
const AI: Color = Color::Rgb(0xc6, 0x78, 0xdd);
const AI_BG: Color = Color::Rgb(0x2d, 0x1f, 0x3a);

fn spin(app: &App) -> &'static str {
    SPIN[(app.tick as usize / 2) % SPIN.len()]
}

fn panel(title: Vec<Span<'static>>, focused: bool) -> Block<'static> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(if focused { ACCENT } else { Color::Rgb(0x3b, 0x41, 0x4b) }))
        .title(Line::from(title))
}

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(DIM))
}

fn bold(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::new().add_modifier(Modifier::BOLD))
}

fn fg(s: impl Into<String>, c: Color) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(c))
}

/// Resalta una línea seleccionada rellenándola a todo el ancho.
fn row(mut spans: Vec<Span<'static>>, w: usize, sel: bool, focused: bool) -> Line<'static> {
    let used: usize = spans.iter().map(|s| width(&s.content)).sum();
    if used < w {
        spans.push(Span::raw(" ".repeat(w - used)));
    }
    let mut l = Line::from(spans);
    if sel {
        l = l.style(Style::new().bg(if focused { SEL_BG } else { SEL_BG_DIM }));
    }
    l
}

pub fn draw(f: &mut Frame, app: &mut App) {
    app.hits.clear();
    let area = f.area();
    if app.picker.is_some() {
        draw_picker(f, app, area);
    } else if app.repo.is_some() {
        let [head, body, foot] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1)]).areas(area);
        draw_header(f, app, head);
        match app.tab {
            Tab::Graph => draw_graph(f, app, body),
            Tab::Mrs => draw_mrs(f, app, body),
            Tab::Files => draw_files(f, app, body),
        }
        draw_footer(f, app, foot);
    }
    if app.help {
        draw_help(f, app, area);
    }
}

// ─────────────────────────────── cabecera ───────────────────────────────

fn draw_header(f: &mut Frame, app: &mut App, area: Rect) {
    let r = app.repo.as_ref().unwrap();
    let host = r
        .remote
        .as_deref()
        .map(|u| {
            u.trim_end_matches(".git")
                .replace("https://", "")
                .replace("http://", "")
                .replace("git@", "")
                .replacen(':', "/", 1)
        })
        .unwrap_or_default();
    let mut left = vec![
        Span::styled(" ◆ gitview ", Style::new().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        bold(r.name.clone()),
        fg(format!("  ⎇ {}", r.branch), lane_color(0)),
        dim(format!("  {host}")),
    ];
    let narrow = area.width < 120;
    let mr_tab = if narrow { format!(" 2 {}s ", r.forge_hint().short()) } else { format!(" 2 {} ", r.forge_hint().label()) };
    let mut tabs = vec![(Tab::Graph, " 1 Grafo ")];
    if r.forge.flatten().is_some() {
        tabs.push((Tab::Mrs, mr_tab.as_str()));
    }
    tabs.push((Tab::Files, " 3 Ficheros "));
    let btns = [
        (Btn::Reveal, if narrow { " ▤ " } else { " ▤ ubicación " }),
        (Btn::Back, if narrow { " ⌂ " } else { " ⌂ proyectos " }),
        (Btn::Refresh, " ↻ "),
        (Btn::Help, " ? "),
        (Btn::Quit, " ✕ "),
    ];
    let tabs_w: usize = tabs.iter().map(|t| width(t.1) + 1).sum::<usize>() + btns.iter().map(|b| width(b.1)).sum::<usize>();
    let left_w: usize = left.iter().map(|s| width(&s.content)).sum();
    let avail = (area.width as usize).saturating_sub(tabs_w);
    if left_w > avail {
        // Recortar por la derecha span a span hasta que quepa (los clics dependen de ello).
        let mut acc = 0;
        let mut kept = Vec::new();
        for sp in left.drain(..) {
            let w = width(&sp.content);
            if acc + w <= avail {
                acc += w;
                kept.push(sp);
            } else {
                let room = avail - acc;
                if room > 1 {
                    kept.push(Span::styled(trunc(&sp.content, room), sp.style));
                }
                break;
            }
        }
        left = kept;
    }
    let lw: usize = left.iter().map(|s| width(&s.content)).sum();
    left.push(Span::raw(" ".repeat(avail.saturating_sub(lw))));
    let mut x = area.x + avail.min(area.width as usize) as u16;
    for (t, label) in tabs {
        let st = if app.tab == t {
            Style::new().bg(SEL_BG).fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(DIM)
        };
        left.push(Span::styled(label, st));
        left.push(Span::raw(" "));
        let w = width(label) as u16;
        app.hits.push((Rect::new(x, area.y, w, 1), Hit::Tab(t)));
        x += w + 1;
    }
    for (b, label) in btns {
        left.push(fg(label, if b == Btn::Quit { DEL } else { DIM }));
        let w = width(label) as u16;
        app.hits.push((Rect::new(x, area.y, w, 1), Hit::Btn(b)));
        x += w;
    }
    f.render_widget(Paragraph::new(Line::from(left)).style(Style::new().bg(Color::Rgb(0x1b, 0x1f, 0x26))), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let hints: &[(&str, &str)] = if app.diff().is_some() && app.focus == Pane::Right {
        &[("↑↓", "desplazar"), ("n/N", "hunk"), ("[ ]", "fichero"), ("e", "explicar fichero"), ("esc", "cerrar diff"), ("q", "salir")]
    } else {
        match app.tab {
            Tab::Graph => &[("↑↓", "mover"), ("⏎", "abrir"), ("⇥", "panel"), ("e", "explicar IA"), ("O", "ubicación"), ("y", "copiar sha"), ("p", "proyectos"), ("?", "ayuda")],
            Tab::Mrs => &[("↑↓", "mover"), ("⏎", "ficheros"), ("e", if app.repo.as_ref().is_some_and(|r| r.forge_hint().short() == "PR") { "explicar PR" } else { "explicar MR" }), ("a", "estado"), ("o", "abrir web"), ("O", "ubicación"), ("?", "ayuda")],
            Tab::Files => &[("↑↓", "mover"), ("⏎", "entrar"), ("⌫", "subir"), ("e", "explicar"), ("o", "abrir repo"), ("O", "ubicación"), ("?", "ayuda")],
        }
    };
    let mut spans = vec![Span::raw(" ")];
    for (k, d) in hints {
        spans.push(Span::styled(*k, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)));
        spans.push(dim(format!(" {d}  ")));
    }
    if let Some((t, _)) = &app.toast {
        let used: usize = spans.iter().map(|s| width(&s.content)).sum();
        let tw = width(t) + 2;
        if (area.width as usize) > used + tw {
            spans.push(Span::raw(" ".repeat(area.width as usize - used - tw)));
        }
        spans.push(Span::styled(format!(" {t} "), Style::new().fg(Color::Black).bg(ADD)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(Style::new().bg(Color::Rgb(0x1b, 0x1f, 0x26))), area);
}

// ──────────────────────────────── grafo ────────────────────────────────

fn split_lr(app: &App, area: Rect) -> (Rect, Rect) {
    let pct = if app.diff().is_some() { 36 } else { 56 };
    let [l, r] = Layout::horizontal([Constraint::Percentage(pct), Constraint::Percentage(100 - pct)]).areas(area);
    (l, r)
}

fn draw_graph(f: &mut Frame, app: &mut App, area: Rect) {
    let (left, right) = split_lr(app, area);
    let focused = app.focus == Pane::Left;
    let len = app.graph_len();
    let r = app.repo.as_ref().unwrap();
    let mut title = vec![bold(" Commits "), dim(format!("{}{} ", r.commits.len(), if r.log_done { "" } else { "+" }))];
    if r.log_loading {
        title.push(fg(format!("{} ", spin(app)), ACCENT));
    }
    let block = panel(title, focused);
    let inner = block.inner(left);
    f.render_widget(block, left);
    let h = inner.height as usize;
    app.repo.as_mut().unwrap().g_nav.fit(h, len);
    let r = app.repo.as_ref().unwrap();
    let (off, sel) = (r.g_nav.off, r.g_nav.sel);
    let w = inner.width as usize;
    let wip = app.wip_rows();
    let now = now();
    let mut lines = Vec::with_capacity(h);
    if let Some(e) = &r.log_err {
        lines.push(Line::from(fg(format!(" Error leyendo el historial: {e}"), DEL)));
    }
    for i in off..(off + h).min(len) {
        let is_sel = i == sel;
        if wip == 1 && i == 0 {
            let spans = vec![
                fg(" ◌ ", WARN),
                Span::styled(format!("Cambios sin commit ({})", r.status.len()), Style::new().fg(WARN).add_modifier(Modifier::ITALIC)),
            ];
            lines.push(row(spans, w, is_sel, focused));
            continue;
        }
        let ci = i - wip;
        let c = &r.commits[ci];
        let mut spans: Vec<Span> = Vec::new();
        let mut used = 0;
        if let Some(g) = r.rows.get(ci) {
            for cell in &g.cells {
                let mut st = Style::new().fg(lane_color(cell.color));
                if cell.ch == '●' || cell.ch == '◉' || cell.ch == '○' {
                    st = st.add_modifier(Modifier::BOLD);
                }
                spans.push(Span::styled(cell.ch.to_string(), st));
            }
            used += g.cells.len();
        }
        spans.push(Span::raw(" "));
        used += 1;
        let right_w = if w >= 100 { 26 } else if w >= 70 { 11 } else { 0 };
        let mid = w.saturating_sub(used + right_w);
        let node_color = r.rows.get(ci).map_or(0, |g| g.color);
        let mut mid_used = 0;
        for rf in &c.refs {
            let (txt, st) = match rf.kind {
                RefKind::Head => continue,
                RefKind::LocalBranch => {
                    let is_head = c.refs.iter().any(|x| x.kind == RefKind::Head && x.name == rf.name)
                        || (r.branch == rf.name);
                    let st = if is_head {
                        Style::new().bg(lane_color(node_color)).fg(Color::Black).add_modifier(Modifier::BOLD)
                    } else {
                        Style::new().fg(lane_color(node_color)).add_modifier(Modifier::BOLD)
                    };
                    (format!("{}{} ", if is_head { " ●" } else { "⎇ " }, rf.name), st)
                }
                RefKind::RemoteBranch => (format!("☁ {} ", rf.name), Style::new().fg(DIM).add_modifier(Modifier::ITALIC)),
                RefKind::Tag => (format!("◇ {} ", rf.name), Style::new().fg(WARN)),
            };
            if mid_used + width(&txt) + 8 > mid {
                break;
            }
            mid_used += width(&txt) + 1;
            spans.push(Span::styled(txt, st));
            spans.push(Span::raw(" "));
        }
        let subj_w = mid.saturating_sub(mid_used);
        spans.push(Span::raw(pad(&c.subject, subj_w)));
        if right_w >= 26 {
            spans.push(dim(format!(" {}", pad(&c.author, 13))));
            spans.push(dim(pad_left(&fmt_short(c.time, now), 11)));
        } else if right_w > 0 {
            spans.push(dim(pad_left(&fmt_short(c.time, now), 11)));
        }
        lines.push(row(spans, w, is_sel, focused));
    }
    if len == 0 && r.log_loading {
        lines.push(Line::from(dim(format!(" {} Cargando historial…", spin(app)))));
    } else if len == 0 && r.log_err.is_none() {
        lines.push(Line::from(dim(" Repositorio sin commits")));
    }
    f.render_widget(Paragraph::new(lines), inner);
    app.hits.push((inner, Hit::List(ListId::Commits)));

    let right = draw_ai_region(f, app, right);
    if right.height < 3 {
        return;
    }
    if app.diff().is_some() {
        let n = app.graph_files().and_then(|r| r.ok()).map_or(0, |v| v.len());
        let (strip, rest) = diff_split(right, n);
        draw_commit_detail(f, app, strip, true);
        draw_diff(f, app, rest);
    } else {
        draw_commit_detail(f, app, right, false);
    }
}

/// Con un diff abierto: franja compacta con la lista de ficheros encima y el diff debajo.
fn diff_split(area: Rect, n: usize) -> (Rect, Rect) {
    let h = (n.clamp(1, 6) as u16 + 2).min(area.height / 3);
    let [strip, rest] = Layout::vertical([Constraint::Length(h), Constraint::Min(3)]).areas(area);
    (strip, rest)
}

fn draw_commit_detail(f: &mut Frame, app: &mut App, area: Rect, compact: bool) {
    let focused = app.focus == Pane::Right && !compact;
    let w = area.width.saturating_sub(2) as usize;
    let mut info: Vec<Line> = Vec::new();
    let r = app.repo.as_ref().unwrap();
    match app.graph_sel() {
        None => info.push(Line::from(dim(" Sin selección"))),
        Some(None) => {
            info.push(Line::from(vec![fg(" ◌ Cambios sin commit", WARN), dim(format!("  en {}", r.branch))]));
            info.push(Line::from(dim(" Pulsa e para que la IA resuma lo que hay pendiente.")));
        }
        Some(Some(i)) => {
            let c = &r.commits[i];
            let d = r.details.get(&c.id);
            info.push(Line::from(vec![
                Span::styled(format!(" {} ", c.short), Style::new().fg(WARN).add_modifier(Modifier::BOLD)),
                bold(c.author.clone()),
                dim(format!("  {}", fmt_date(c.time))),
            ]));
            if c.parents.len() > 1 {
                info.push(Line::from(dim(format!(
                    " merge de {}",
                    c.parents.iter().map(|p| &p[..p.len().min(8)]).collect::<Vec<_>>().join(" + ")
                ))));
            }
            let msg = d.map(|d| d.message.clone()).unwrap_or_else(|| c.subject.clone());
            let mut ml = wrap(msg.trim(), w.saturating_sub(1)).into_iter();
            if let Some(first) = ml.next() {
                info.push(Line::from(Span::styled(format!(" {first}"), Style::new().fg(Color::White).add_modifier(Modifier::BOLD))));
            }
            for l in ml.filter(|l| !l.trim().is_empty()).take(6) {
                info.push(Line::from(dim(format!(" {l}"))));
            }
        }
    }
    let files = app.graph_files();
    let n = files.as_ref().and_then(|f| f.as_ref().ok()).map_or(0, |v| v.len());
    let info_h = if compact { 0 } else { (info.len() as u16 + 1).min(area.height / 2).max(3) };
    let [top, bottom] = Layout::vertical([Constraint::Length(info_h), Constraint::Min(3)]).areas(area);
    f.render_widget(Paragraph::new(info), top);

    let (adds, dels) = files
        .as_ref()
        .and_then(|f| f.as_ref().ok())
        .map(|v| v.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions)))
        .unwrap_or((0, 0));
    let block = panel(
        vec![bold(" Ficheros "), dim(format!("{n} ")), fg(format!("+{adds} "), ADD), fg(format!("−{dels} "), DEL)],
        focused,
    );
    let inner = block.inner(bottom);
    f.render_widget(block, bottom);
    let h = inner.height as usize;
    app.repo.as_mut().unwrap().g_files_nav.fit(h, n);
    let nav = app.repo.as_ref().unwrap().g_files_nav.clone();
    let lines: Vec<Line> = match &files {
        None => vec![Line::from(dim(format!(" {} cargando…", spin(app))))],
        Some(Err(e)) => vec![Line::from(fg(format!(" {e}"), DEL))],
        Some(Ok(v)) => v
            .iter()
            .enumerate()
            .skip(nav.off)
            .take(h)
            .map(|(i, fc)| file_row(fc, inner.width as usize, i == nav.sel, focused))
            .collect(),
    };
    f.render_widget(Paragraph::new(lines), inner);
    app.hits.push((inner, Hit::List(ListId::CommitFiles)));
}

fn file_row(fc: &FileChange, w: usize, sel: bool, focused: bool) -> Line<'static> {
    let stat = if fc.binary {
        "bin".to_string()
    } else if fc.additions + fc.deletions == 0 {
        String::new()
    } else {
        format!("+{} −{}", fc.additions, fc.deletions)
    };
    let name = match &fc.old_path {
        Some(o) if o != &fc.path => format!("{o} → {}", fc.path),
        _ => fc.path.clone(),
    };
    let name_w = w.saturating_sub(4 + width(&stat) + 1);
    let shown = trunc_left(&name, name_w);
    let (dir, file) = match shown.rfind('/') {
        Some(p) => (shown[..=p].to_string(), shown[p + 1..].to_string()),
        None => (String::new(), shown.clone()),
    };
    let gap = name_w.saturating_sub(width(&shown));
    let mut spans = vec![
        Span::styled(format!(" {} ", fc.status), Style::new().fg(status_color(fc.status)).add_modifier(Modifier::BOLD)),
        dim(dir),
        Span::raw(file),
        Span::raw(" ".repeat(gap + 1)),
    ];
    if !stat.is_empty() && !fc.binary {
        let (a, d) = stat.split_once(' ').unwrap_or((&stat, ""));
        spans.push(fg(a.to_string(), ADD));
        spans.push(fg(format!(" {d}"), DEL));
    } else {
        spans.push(dim(stat));
    }
    row(spans, w, sel, focused)
}

/// Recorta por la izquierda ("…ruta/larga/fichero.rs").
fn trunc_left(s: &str, w: usize) -> String {
    if width(s) <= w || w < 2 {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = Vec::new();
    let mut acc = 0;
    for c in chars.iter().rev() {
        let cw = width(&c.to_string());
        if acc + cw > w - 1 {
            break;
        }
        acc += cw;
        out.push(*c);
    }
    out.reverse();
    format!("…{}", out.into_iter().collect::<String>())
}

// ───────────────────────────────── diff ─────────────────────────────────

fn draw_diff(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Pane::Right;
    let dv = app.diff().unwrap();
    let block = panel(
        vec![bold(format!(" {} ", dv.title)), fg(format!("+{} ", dv.adds), ADD), fg(format!("−{} ", dv.dels), DEL)],
        focused,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    // Botón cerrar en la esquina.
    let close = Rect::new(area.x + area.width.saturating_sub(6), area.y, 5.min(area.width), 1);
    if area.width > 12 {
        f.render_widget(Paragraph::new(Span::styled(" ✕ ", Style::new().fg(DEL))), close);
    }
    let h = inner.height as usize;
    app.diff_h = h;
    let dv = app.diff_mut().unwrap();
    dv.scroll = dv.scroll.min(dv.lines.len().saturating_sub(h.max(1)));
    let w = inner.width as usize;
    let num_w = dv.lines.iter().filter_map(|l| l.new.max(l.old)).max().unwrap_or(0).to_string().len().max(3);
    let text_w = w.saturating_sub(num_w * 2 + 3);
    let lines: Vec<Line> = dv
        .lines
        .iter()
        .skip(dv.scroll)
        .take(h)
        .map(|l| {
            let num = |n: Option<u32>| n.map_or(" ".repeat(num_w), |n| pad_left(&n.to_string(), num_w));
            let (st, sign) = match l.kind {
                Kind::Add => (Style::new().fg(ADD).bg(ADD_BG), "+"),
                Kind::Del => (Style::new().fg(DEL).bg(DEL_BG), "-"),
                Kind::Hunk => (Style::new().fg(HUNK), ""),
                Kind::Meta => (Style::new().fg(DIM).add_modifier(Modifier::BOLD), ""),
                Kind::Ctx => (Style::new(), " "),
            };
            if matches!(l.kind, Kind::Hunk | Kind::Meta) {
                return Line::from(Span::styled(pad(&l.text, w), st));
            }
            Line::from(vec![
                dim(num(l.old)),
                dim(" "),
                dim(num(l.new)),
                Span::styled(format!(" {sign}"), st),
                Span::styled(pad(&l.text, text_w), st),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
    app.hits.push((inner, Hit::List(ListId::Diff)));
    app.hits.push((close, Hit::Btn(Btn::CloseDiff)));
}

// ────────────────────────────────── IA ──────────────────────────────────

/// Reserva la parte inferior para la IA (caja o botón) y devuelve el área restante.
fn draw_ai_region(f: &mut Frame, app: &mut App, area: Rect) -> Rect {
    let Some((job, st)) = app.current_ai() else { return area };
    // 0 = esperando, 3 = llegando texto, 1 = hecho, 2 = error.
    let state = st.map(|s| match s {
        AiState::Running(t, partial) if !partial.is_empty() => (3u8, partial.clone(), t.elapsed().as_secs()),
        AiState::Running(t, _) => (0, String::new(), t.elapsed().as_secs()),
        AiState::Done(t) => (1, t.clone(), 0),
        AiState::Failed(e) => (2, e.clone(), 0),
    });
    let Some((kind, body, secs)) = state else {
        let [top, btn] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
        let label = format!(" ✨ Explicar {} con IA  (e) ", job.label);
        let lw = (width(&label) as u16).min(btn.width);
        let r = Rect::new(btn.x, btn.y, lw, 1);
        f.render_widget(Paragraph::new(Span::styled(label, Style::new().fg(Color::White).bg(AI_BG).add_modifier(Modifier::BOLD))), r);
        app.hits.push((r, Hit::Btn(Btn::Explain)));
        return top;
    };
    let pct = if app.ai_max { 100 } else if kind == 1 || kind == 3 { 50 } else { 25 };
    let [top, bottom] = Layout::vertical([Constraint::Percentage(100 - pct), Constraint::Percentage(pct)]).areas(area);
    let mut title = vec![Span::styled(" ✨ IA ", Style::new().fg(AI).add_modifier(Modifier::BOLD)), dim(format!("· {} ", job.label))];
    if kind == 0 || kind == 3 {
        title.push(fg(format!("{} ", spin(app)), AI));
    }
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(AI))
        .title(Line::from(title));
    let inner = block.inner(bottom);
    let w = inner.width.saturating_sub(1) as usize;
    let lines: Vec<Line> = match kind {
        0 => vec![Line::from(vec![fg(format!(" {} ", spin(app)), AI), dim(format!("Analizando cambios… {secs}s"))])],
        1 => markdown(&body, w),
        3 => {
            let mut v = markdown(&body, w);
            v.push(Line::from(fg(format!(" {}", spin(app)), AI)));
            v
        }
        _ => {
            let mut v = vec![Line::from(fg(" No se pudo generar la explicación:", DEL))];
            v.extend(wrap(&body, w).into_iter().map(|l| Line::from(dim(format!(" {l}")))));
            v.push(Line::from(dim(" Pulsa e para reintentar (comando en ~/.config/herdr-gitview/config.toml).")));
            v
        }
    };
    let h = inner.height as usize;
    let total = lines.len();
    app.ai_nav.off = app.ai_nav.off.min(total.saturating_sub(h));
    let off = app.ai_nav.off;
    // Pie del recuadro: cuánto queda arriba/abajo y cómo moverse; botón ampliar/reducir.
    let below = total.saturating_sub(off + h);
    let narrow = bottom.width < 56;
    let mut foot = Vec::new();
    if off > 0 || below > 0 {
        let arrows = match (off > 0, below > 0, narrow) {
            (true, true, _) => format!(" ↑{off} ↓{below} "),
            (true, false, _) => format!(" ↑{off} "),
            (_, _, true) => format!(" ↓{below} "),
            _ => format!(" ↓ {below} líneas más "),
        };
        foot.push(Span::styled(arrows, Style::new().fg(Color::Black).bg(AI)));
        foot.push(dim(if narrow { " J/K " } else { " J/K · rueda " }));
    }
    let zoom = match (app.ai_max, narrow) {
        (true, true) => " ⤡z ",
        (false, true) => " ⤢z ",
        (true, false) => " ⤡ reducir (z) ",
        (false, false) => " ⤢ ampliar (z) ",
    };
    foot.push(Span::styled(zoom, Style::new().fg(AI)));
    let block = block.title_bottom(Line::from(foot).right_aligned());
    f.render_widget(block, bottom);
    let zw = width(zoom) as u16;
    if bottom.width > zw + 2 && bottom.height > 1 {
        let zr = Rect::new(bottom.x + bottom.width - 1 - zw, bottom.y + bottom.height - 1, zw, 1);
        app.hits.push((zr, Hit::Btn(Btn::AiMax)));
    }
    // Barra de desplazamiento en el borde derecho.
    if total > h && h > 0 {
        let x = bottom.x + bottom.width - 1;
        let thumb = (h * h / total).max(1);
        let pos = (h - thumb) * off / total.saturating_sub(h).max(1);
        for i in 0..thumb {
            let cell = Rect::new(x, inner.y + (pos + i) as u16, 1, 1);
            f.render_widget(Paragraph::new(Span::styled("┃", Style::new().fg(AI))), cell);
        }
    }
    let shown: Vec<Line> = lines.into_iter().skip(off).collect();
    f.render_widget(Paragraph::new(shown), inner);
    app.hits.push((inner, Hit::List(ListId::Ai)));
    top
}

// ───────────────────────────── merge requests ─────────────────────────────

fn mr_badge(state: &str) -> Span<'static> {
    match state {
        "opened" => fg("● ", ADD),
        "merged" => fg("✓ ", MERGED),
        "closed" => fg("✕ ", DEL),
        _ => fg("○ ", DIM),
    }
}

fn draw_mrs(f: &mut Frame, app: &mut App, area: Rect) {
    let (left, right) = split_lr(app, area);
    let focused = app.focus == Pane::Left;
    let r = app.repo.as_ref().unwrap();
    let forge = r.forge_hint();
    let mut title = vec![bold(format!(" {} ", forge.label())), dim(format!("{} ", r.mrs.len()))];
    if r.mr_loading || r.forge.is_none() {
        title.push(fg(format!("{} ", spin(app)), ACCENT));
    }
    let block = panel(title, focused);
    let inner = block.inner(left);
    f.render_widget(block, left);
    if inner.height < 2 {
        return;
    }
    // Filtros de estado clicables.
    let chips = [(MrState::Opened, "Abiertos"), (MrState::Merged, "Fusionados"), (MrState::Closed, "Cerrados"), (MrState::All, "Todos")];
    let mut x = inner.x + 1;
    let mut spans = vec![Span::raw(" ")];
    for (st, label) in chips {
        let txt = format!(" {label} ");
        let on = r.mr_state == st;
        spans.push(Span::styled(
            txt.clone(),
            if on { Style::new().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD) } else { Style::new().fg(DIM).bg(SEL_BG_DIM) },
        ));
        spans.push(Span::raw(" "));
        let w = width(&txt) as u16;
        app.hits.push((Rect::new(x, inner.y, w, 1), Hit::Btn(Btn::MrState(st))));
        x += w + 1;
    }
    f.render_widget(Paragraph::new(Line::from(spans)), Rect::new(inner.x, inner.y, inner.width, 1));
    let list = Rect::new(inner.x, inner.y + 2, inner.width, inner.height.saturating_sub(2));
    let h = list.height as usize;
    let len = r.mrs.len();
    app.repo.as_mut().unwrap().mr_nav.fit(h, len);
    let r = app.repo.as_ref().unwrap();
    let (off, sel) = (r.mr_nav.off, r.mr_nav.sel);
    let w = list.width as usize;
    let now = now();
    let mut lines = Vec::new();
    match r.forge {
        None => lines.push(Line::from(dim(format!(" {} comprobando {}...", spin(app), forge.name())))),
        Some(None) => {
            lines.push(Line::from(fg(format!(" Este repositorio no apunta a un {} con {} autenticado.", forge.name(), forge.cli()), WARN)));
            lines.push(Line::from(dim(format!(" Comprueba `{} auth status` y el remoto `origin`.", forge.cli()))));
        }
        Some(Some(forge)) => {
            if let Some(e) = &r.mr_err {
                for l in wrap(e, w.saturating_sub(2)) {
                    lines.push(Line::from(fg(format!(" {l}"), DEL)));
                }
            }
            for (i, mr) in r.mrs.iter().enumerate().skip(off).take(h) {
                let id = format!("{}{:<5}", forge.id_prefix(), mr.iid);
                let right = if w >= 80 { format!(" {} {}", pad(&mr.author, 14), pad_left(&fmt_short(mr.updated_at, now), 7)) } else { String::new() };
                let tw = w.saturating_sub(width(&id) + 3 + width(&right));
                let title = if mr.draft { format!("Draft: {}", mr.title) } else { mr.title.clone() };
                let spans = vec![
                    Span::raw(" "),
                    mr_badge(&mr.state),
                    fg(id, ACCENT),
                    Span::styled(pad(&title, tw), if mr.draft { Style::new().fg(DIM) } else { Style::new() }),
                    dim(right),
                ];
                lines.push(row(spans, w, i == sel, focused));
            }
            if r.mrs.is_empty() && !r.mr_loading && r.mr_err.is_none() {
                lines.push(Line::from(dim(format!(" No hay {}s en este estado.", forge.short()))));
            }
        }
    }
    f.render_widget(Paragraph::new(lines), list);
    app.hits.push((list, Hit::List(ListId::Mrs)));

    let right = draw_ai_region(f, app, right);
    if right.height < 3 {
        return;
    }
    if app.diff().is_some() {
        let n = app
            .sel_mr()
            .and_then(|m| app.repo.as_ref()?.mr_diffs.get(&m.iid))
            .and_then(|r| r.as_ref().ok())
            .map_or(0, |v| v.len());
        let (strip, rest) = diff_split(right, n);
        draw_mr_detail(f, app, strip, true);
        draw_diff(f, app, rest);
    } else {
        draw_mr_detail(f, app, right, false);
    }
}

fn draw_mr_detail(f: &mut Frame, app: &mut App, area: Rect, compact: bool) {
    let focused = app.focus == Pane::Right && !compact;
    let w = area.width.saturating_sub(2) as usize;
    let Some(mr) = app.sel_mr().cloned() else {
        let label = app.repo.as_ref().map_or("solicitud", |r| r.forge_hint().short());
        f.render_widget(Paragraph::new(dim(format!(" Selecciona un {label}"))), area);
        return;
    };
    let r = app.repo.as_ref().unwrap();
    let mut info: Vec<Line> = Vec::new();
    for (i, l) in wrap(&mr.title, w).into_iter().enumerate() {
        let pre = if i == 0 { format!(" {}{} ", r.forge_hint().id_prefix(), mr.iid) } else { "  ".into() };
        info.push(Line::from(vec![fg(pre, ACCENT), Span::styled(l, Style::new().fg(Color::White).add_modifier(Modifier::BOLD))]));
    }
    let mut meta = vec![Span::raw(" "), mr_badge(&mr.state), Span::raw(format!("{}  ", mr.state)), bold(mr.author.clone())];
    meta.push(dim(format!("  {}", fmt_date(mr.merged_at.unwrap_or(mr.updated_at)))));
    if let Some(p) = &mr.pipeline_status {
        let c = match p.as_str() {
            "success" => ADD,
            "failed" => DEL,
            "running" | "pending" => WARN,
            _ => DIM,
        };
        meta.push(fg(format!("  ⚙ {p}"), c));
    }
    info.push(Line::from(meta));
    let web_row = info.len() as u16;
    info.push(Line::from(vec![
        Span::raw(" "),
        fg(format!("⎇ {}", mr.source_branch), lane_color(1)),
        dim(" → "),
        fg(format!("⎇ {}", mr.target_branch), lane_color(0)),
        dim(if mr.user_notes_count > 0 { format!("   💬 {}", mr.user_notes_count) } else { String::new() }),
    ]));
    if !mr.labels.is_empty() {
        info.push(Line::from(fg(format!(" {}", mr.labels.iter().map(|l| format!("[{l}]")).collect::<Vec<_>>().join(" ")), WARN)));
    }
    if let Some(cs) = r.mr_commits.get(&mr.iid) {
        info.push(Line::from(dim(format!(" {} commit{}", cs.len(), if cs.len() == 1 { "" } else { "s" }))));
        for c in cs.iter().take(4) {
            info.push(Line::from(vec![fg(format!("   {} ", c.short), WARN), Span::raw(trunc(&c.title, w.saturating_sub(12)))]));
        }
    }
    let desc = mr.description.trim();
    if !desc.is_empty() {
        info.push(Line::from(""));
        for l in wrap(desc, w.saturating_sub(1)).into_iter().filter(|l| !l.trim().is_empty()).take(8) {
            info.push(Line::from(dim(format!(" {l}"))));
        }
    }
    let info_h = if compact { 0 } else { (info.len() as u16 + 1).min(area.height / 2).max(4) };
    let [top, bottom] = Layout::vertical([Constraint::Length(info_h), Constraint::Min(3)]).areas(area);
    f.render_widget(Paragraph::new(info), top);
    let label = format!(" {} (o) ", r.forge_hint().name());
    let lw = width(&label) as u16;
    if web_row < top.height && top.width > lw + 40 {
        let b = Rect::new(top.x + top.width - lw - 1, top.y + web_row, lw, 1);
        f.render_widget(Paragraph::new(Span::styled(label, Style::new().fg(ACCENT).bg(SEL_BG_DIM))), b);
        app.hits.push((b, Hit::Btn(Btn::OpenWeb)));
    }

    let diffs = r.mr_diffs.get(&mr.iid).cloned();
    let n = diffs.as_ref().and_then(|d| d.as_ref().ok()).map_or(0, |v| v.len());
    let (adds, dels) =
        diffs.as_ref().and_then(|d| d.as_ref().ok()).map(|v| v.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions))).unwrap_or((0, 0));
    let block = panel(
        vec![bold(" Ficheros "), dim(format!("{n} ")), fg(format!("+{adds} "), ADD), fg(format!("−{dels} "), DEL)],
        focused,
    );
    let inner = block.inner(bottom);
    f.render_widget(block, bottom);
    let h = inner.height as usize;
    app.repo.as_mut().unwrap().mr_files_nav.fit(h, n);
    let nav = app.repo.as_ref().unwrap().mr_files_nav.clone();
    let lines: Vec<Line> = match &diffs {
        None => vec![Line::from(dim(format!(" {} cargando cambios…", spin(app))))],
        Some(Err(e)) => vec![Line::from(fg(format!(" {e}"), DEL))],
        Some(Ok(v)) => v
            .iter()
            .enumerate()
            .skip(nav.off)
            .take(h)
            .map(|(i, d)| {
                let fc = FileChange {
                    path: d.new_path.clone(),
                    old_path: d.renamed_file.then(|| d.old_path.clone()),
                    status: if d.new_file { 'A' } else if d.deleted_file { 'D' } else if d.renamed_file { 'R' } else { 'M' },
                    additions: d.additions,
                    deletions: d.deletions,
                    binary: false,
                };
                file_row(&fc, inner.width as usize, i == nav.sel, focused)
            })
            .collect(),
    };
    f.render_widget(Paragraph::new(lines), inner);
    app.hits.push((inner, Hit::List(ListId::MrFiles)));
}

// ─────────────────────────────── ficheros ───────────────────────────────

fn home_rel(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().into_owned();
    match std::env::var("HOME") {
        Ok(h) if s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}

fn draw_files(f: &mut Frame, app: &mut App, area: Rect) {
    let (left, right) = split_lr(app, area);
    let focused = app.focus == Pane::Left;
    let r = app.repo.as_ref().unwrap();
    let block = panel(vec![bold(" Explorador "), dim(format!("{} ", home_rel(&r.fs_dir)))], focused);
    let inner = block.inner(left);
    f.render_widget(block, left);
    let h = inner.height as usize;
    let len = r.fs_entries.len();
    app.repo.as_mut().unwrap().fs_nav.fit(h, len);
    let r = app.repo.as_ref().unwrap();
    let w = inner.width as usize;
    let lines: Vec<Line> = r
        .fs_entries
        .iter()
        .enumerate()
        .skip(r.fs_nav.off)
        .take(h)
        .map(|(i, e)| {
            let (icon, st) = if e.name == ".." {
                ("↰ ", Style::new().fg(DIM))
            } else if e.is_repo {
                ("◆ ", Style::new().fg(lane_color(2)).add_modifier(Modifier::BOLD))
            } else if e.is_dir {
                ("▸ ", Style::new().fg(ACCENT))
            } else {
                ("  ", Style::new())
            };
            let mark = e.mark.map(|c| format!("{c} ")).unwrap_or_default();
            let name = if e.is_dir && e.name != ".." { format!("{}/", e.name) } else { e.name.clone() };
            let nw = w.saturating_sub(3 + width(&mark));
            let mut spans = vec![Span::raw(" "), Span::styled(icon, st), Span::styled(pad(&name, nw), if e.is_dir { st } else { Style::new() })];
            if let Some(c) = e.mark {
                spans.push(Span::styled(mark, Style::new().fg(if c == '•' { WARN } else { status_color(c) }).add_modifier(Modifier::BOLD)));
            }
            row(spans, w, i == r.fs_nav.sel, focused)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
    app.hits.push((inner, Hit::List(ListId::Fs)));

    let right = if app.sel_fs_rel().is_some() { draw_ai_region(f, app, right) } else { right };
    if right.height < 3 {
        return;
    }
    let mut right = right;
    let compact = app.diff().is_some();
    if compact {
        let (wip, hist) = app.fs_hist_rows();
        let n = usize::from(wip) + hist.and_then(|h| h.as_ref().ok()).map_or(0, |v| v.len());
        let (strip, rest) = diff_split(right, n);
        draw_diff(f, app, rest);
        right = strip;
    }
    let focused = app.focus == Pane::Right && !compact;
    let Some(e) = app.sel_fs().cloned() else { return };
    if e.is_dir {
        let mut lines = vec![Line::from(vec![Span::raw(" "), bold(home_rel(&e.path))])];
        if e.is_repo && e.path != app.repo.as_ref().unwrap().root {
            lines.push(Line::from(""));
            lines.push(Line::from(fg(" Es un repositorio git.", lane_color(2))));
            f.render_widget(Paragraph::new(lines), right);
            let label = " ◆ Abrir su control de cambios  (o) ";
            let b = Rect::new(right.x + 1, right.y + 3, (width(label) as u16).min(right.width.saturating_sub(1)), 1);
            if right.height < 4 {
                return;
            }
            f.render_widget(Paragraph::new(Span::styled(label, Style::new().bg(SEL_BG).fg(Color::White).add_modifier(Modifier::BOLD))), b);
            app.hits.push((b, Hit::Btn(Btn::OpenRepo)));
        } else {
            if e.mark.is_some() {
                lines.push(Line::from(fg(" Contiene cambios sin commit", WARN)));
            }
            lines.push(Line::from(dim(" ⏎ / doble clic para entrar · ⌫ para subir")));
            f.render_widget(Paragraph::new(lines), right);
        }
        return;
    }
    let Some(rel) = app.sel_fs_rel() else {
        f.render_widget(Paragraph::new(dim(" Fuera del repositorio actual: sin historial.")), right);
        return;
    };
    let (wip, hist) = app.fs_hist_rows();
    let hist = hist.cloned();
    let n = usize::from(wip) + hist.as_ref().and_then(|h| h.as_ref().ok()).map_or(0, |v| v.len());
    let block = panel(vec![bold(" Historial "), dim(format!("{rel} · {n} "))], focused);
    let inner = block.inner(right);
    f.render_widget(block, right);
    let h = inner.height as usize;
    app.repo.as_mut().unwrap().fs_hist_nav.fit(h, n);
    let nav = app.repo.as_ref().unwrap().fs_hist_nav.clone();
    let w = inner.width as usize;
    let now = now();
    let mut lines = Vec::new();
    for i in nav.off..(nav.off + h).min(n) {
        if wip && i == 0 {
            lines.push(row(vec![fg(" ◌ ", WARN), Span::styled("Cambios sin commit", Style::new().fg(WARN).add_modifier(Modifier::ITALIC))], w, i == nav.sel, focused));
            continue;
        }
        let Some(Ok(v)) = &hist else { break };
        let c = &v[i - usize::from(wip)];
        let right_s = format!(" {} {}", pad(&c.author, 12), pad_left(&fmt_short(c.time, now), 7));
        let sw = w.saturating_sub(10 + width(&right_s));
        lines.push(row(
            vec![fg(format!(" {:<8} ", &c.short[..c.short.len().min(8)]), WARN), Span::raw(pad(&c.subject, sw)), dim(right_s)],
            w,
            i == nav.sel,
            focused,
        ));
    }
    match &hist {
        None => lines.push(Line::from(dim(format!(" {} cargando historial…", spin(app))))),
        Some(Err(e)) => lines.push(Line::from(fg(format!(" {e}"), DEL))),
        Some(Ok(v)) if v.is_empty() && !wip => lines.push(Line::from(dim(" Sin historial (fichero no versionado)"))),
        _ => {}
    }
    f.render_widget(Paragraph::new(lines), inner);
    app.hits.push((inner, Hit::List(ListId::FsHist)));
}

// ─────────────────────────────── selector ───────────────────────────────

fn draw_picker(f: &mut Frame, app: &mut App, area: Rect) {
    let p = app.picker.as_ref().unwrap();
    let n_active = p.cands.iter().filter(|c| c.active).count();
    let block = panel(
        vec![
            Span::styled(" ◆ gitview ", Style::new().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD)),
            bold(" ¿Qué proyecto quieres ver? "),
            dim(format!("{} · {} con actividad reciente ", home_rel(&p.base), n_active)),
        ],
        true,
    );
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 4 {
        return;
    }
    let filter = Line::from(vec![
        dim(" Filtrar: "),
        Span::styled(p.filter.clone(), Style::new().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled("▏", Style::new().fg(ACCENT)),
    ]);
    f.render_widget(Paragraph::new(filter), Rect::new(inner.x, inner.y, inner.width, 1));
    let list = Rect::new(inner.x, inner.y + 2, inner.width, inner.height.saturating_sub(3));
    let h = list.height as usize;
    let len = p.visible().len();
    app.picker.as_mut().unwrap().nav.fit(h, len);
    let p = app.picker.as_ref().unwrap();
    let vis = p.visible();
    let w = list.width as usize;
    let name_w = vis.iter().map(|c| width(&c.name)).max().unwrap_or(10).max(12).min(w / 3).max(1);
    let lines: Vec<Line> = vis
        .iter()
        .enumerate()
        .skip(p.nav.off)
        .take(h)
        .map(|(i, c)| {
            let dot = if c.active { fg(" ● ", ADD) } else { dim(" ○ ") };
            let when = c.last_activity.map(crate::discover::ago).unwrap_or_default();
            let br = c.branch.clone().map(|b| format!("⎇ {b}")).unwrap_or_default();
            let reasons = c.reasons.join(" · ");
            let used = 3 + name_w + 2 + 22 + 16;
            let spans = vec![
                dot,
                Span::styled(pad(&c.name, name_w), if c.active { Style::new().fg(Color::White).add_modifier(Modifier::BOLD) } else { Style::new() }),
                Span::raw("  "),
                fg(pad(&br, 22), lane_color(0)),
                dim(pad(&when, 16)),
                dim(trunc(&reasons, w.saturating_sub(used))),
            ];
            row(spans, w, i == p.nav.sel, true)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), list);
    app.hits.push((list, Hit::List(ListId::Picker)));
    let foot = Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            dim(" escribe para filtrar · "),
            fg("⏎", ACCENT),
            dim(" / clic abrir · "),
            fg("Ctrl+O", ACCENT),
            dim(" ver carpeta · "),
            fg("esc", ACCENT),
            dim(" salir · ● = un agente ha trabajado ahí recientemente"),
        ])),
        foot,
    );
}

// ──────────────────────────────── ayuda ────────────────────────────────

fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let forge = app.repo.as_ref().map(|r| r.forge_hint());
    let short = forge.map_or("PR/MR", |f| f.short());
    let tabs_help = if let Some(forge) = forge.filter(|_| app.repo.as_ref().is_some_and(|r| r.forge.flatten().is_some())) {
        format!("Grafo / {} / Ficheros (o clic en la pestaña)", forge.label())
    } else {
        "Grafo / Ficheros (o clic en la pestaña)".into()
    };
    let explain_help = format!("explicar con IA (commit, fichero o {short})");
    let state_help = format!("{short}s: cambiar filtro de estado");
    let open_help = format!("{short}s: abrir en navegador / Ficheros: abrir repo");
    let copy_help = format!("copiar SHA (o URL del {short}) al portapapeles");
    let keys: &[(&str, &str)] = &[
        ("1 / 2 / 3", &tabs_help),
        ("↑↓ j k · rueda", "mover / desplazar"),
        ("PgUp PgDn · g G", "página · inicio / fin"),
        ("⏎ · → · l · doble clic", "abrir (commit → ficheros → diff)"),
        ("⇥", "cambiar de panel"),
        ("esc · ← · ⌫", "volver / cerrar diff / subir carpeta"),
        ("e", &explain_help),
        ("E", "regenerar la explicación (ignora caché)"),
        ("J / K · rueda", "desplazar la explicación"),
        ("z", "ampliar / reducir la caja de IA"),
        ("n / N", "siguiente / anterior hunk en el diff"),
        ("[ / ]", "fichero anterior / siguiente con el diff abierto"),
        ("a", &state_help),
        ("o", &open_help),
        ("O · Ctrl+O", "abrir la ubicación real en el gestor de archivos"),
        ("y", &copy_help),
        ("p", "volver al selector de proyectos"),
        ("r", "recargar"),
        ("q · esc", "salir"),
    ];
    let w = 72.min(area.width.saturating_sub(4));
    let h = (keys.len() as u16 + 4).min(area.height.saturating_sub(2));
    let r = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    f.render_widget(Clear, r);
    let block = panel(vec![bold(" Atajos "), dim("(cualquier tecla para cerrar) ")], true);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let lines: Vec<Line> = keys
        .iter()
        .map(|(k, d)| Line::from(vec![Span::styled(format!(" {}", pad(k, 24)), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)), Span::raw(*d)]))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, forge::Forge};
    use ratatui::{Terminal, backend::TestBackend};

    fn render(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(160, 45)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect()
    }

    #[test]
    fn forge_labels_identifiers_help_and_hidden_tab_render_without_terminal() {
        let mut app = App::new(Config::default());
        app.repo = Some(test_repo(Some(Forge::GitHub)));
        app.tab = Tab::Mrs;
        let screen = render(&mut app);
        assert!(screen.contains("2 Pull Requests"));
        assert!(screen.contains("#12"));
        assert!(screen.contains("GitHub (o)"));
        assert!(!screen.contains("Merge Requests"));
        app.help = true;
        assert!(render(&mut app).contains("PRs: cambiar filtro"));
        app.help = false;
        app.repo.as_mut().unwrap().forge = Some(Some(Forge::GitLab));
        let screen = render(&mut app);
        assert!(screen.contains("2 Merge Requests"));
        assert!(screen.contains("!12"));
        assert!(screen.contains("GitLab (o)"));
        app.repo.as_mut().unwrap().forge = Some(None);
        app.tab = Tab::Graph;
        let screen = render(&mut app);
        assert!(!screen.contains("2 Pull Requests"));
        assert!(!screen.contains("2 Merge Requests"));
        assert!(!app.hits.iter().any(|(_, hit)| matches!(hit, Hit::Tab(Tab::Mrs))));
    }
}
