//! Utilidades de texto: fechas, anchos, markdown mínimo, paleta.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::sync::OnceLock;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Paleta de carriles de Git Graph (VS Code).
const LANES: [(u8, u8, u8); 12] = [
    (0x00, 0x85, 0xd9),
    (0xd9, 0x00, 0x8f),
    (0x00, 0xd9, 0x0a),
    (0xd9, 0x85, 0x00),
    (0xa3, 0x00, 0xd9),
    (0xff, 0x30, 0x30),
    (0x00, 0xd9, 0xcc),
    (0xe1, 0x38, 0xe8),
    (0x85, 0xd9, 0x00),
    (0xdc, 0x5b, 0x23),
    (0x6f, 0x24, 0xd6),
    (0xff, 0xcc, 0x00),
];

pub fn lane_color(i: usize) -> Color {
    let (r, g, b) = LANES[i % LANES.len()];
    Color::Rgb(r, g, b)
}

pub const ACCENT: Color = Color::Rgb(0x4f, 0xa3, 0xff);
pub const DIM: Color = Color::Rgb(0x80, 0x86, 0x90);
pub const SEL_BG: Color = Color::Rgb(0x26, 0x4f, 0x78);
pub const SEL_BG_DIM: Color = Color::Rgb(0x2c, 0x31, 0x3a);
pub const ADD: Color = Color::Rgb(0x3f, 0xb9, 0x50);
pub const DEL: Color = Color::Rgb(0xf8, 0x51, 0x49);
pub const WARN: Color = Color::Rgb(0xd2, 0x99, 0x22);
pub const HUNK: Color = Color::Rgb(0x56, 0xb6, 0xc2);
pub const ADD_BG: Color = Color::Rgb(0x12, 0x2b, 0x1a);
pub const DEL_BG: Color = Color::Rgb(0x35, 0x16, 0x18);

pub fn status_color(c: char) -> Color {
    match c {
        'A' | '?' => ADD,
        'D' => DEL,
        'R' | 'C' => ACCENT,
        'U' => WARN,
        _ => WARN,
    }
}

/// Desfase local en segundos (una vez). Sin dependencias: `date +%z` en Unix y
/// `GetTimeZoneInformation` de kernel32 en Windows.
#[cfg(not(windows))]
fn tz_offset() -> i64 {
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| {
        let out = crate::platform::cmd("date").arg("+%z").output();
        let s = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        if s.len() == 5 {
            let sign = if s.starts_with('-') { -1 } else { 1 };
            let h: i64 = s[1..3].parse().unwrap_or(0);
            let m: i64 = s[3..5].parse().unwrap_or(0);
            sign * (h * 3600 + m * 60)
        } else {
            0
        }
    })
}

/// Desfase local en segundos (una vez) vía `GetTimeZoneInformation`.
#[cfg(windows)]
fn tz_offset() -> i64 {
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(win_tz::offset_secs)
}

#[cfg(windows)]
mod win_tz {
    /// `SYSTEMTIME` (8 campos de 16 bits).
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    /// `TIME_ZONE_INFORMATION` (`minwindef.h`).
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct TimeZoneInformation {
        bias: i32,
        standard_name: [u16; 32],
        standard_date: SystemTime,
        standard_bias: i32,
        daylight_name: [u16; 32],
        daylight_date: SystemTime,
        daylight_bias: i32,
    }

    const TIME_ZONE_ID_INVALID: u32 = 0xffff_ffff;
    const TIME_ZONE_ID_STANDARD: u32 = 1;
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetTimeZoneInformation(info: *mut TimeZoneInformation) -> u32;
    }

    /// Offset local respecto a UTC en segundos: −(Bias + DaylightBias|StandardBias) minutos.
    pub(super) fn offset_secs() -> i64 {
        let mut info = TimeZoneInformation::default();
        // SAFETY: `info` es un `TIME_ZONE_INFORMATION` válido y exclusivo.
        let id = unsafe { GetTimeZoneInformation(&mut info) };
        if id == TIME_ZONE_ID_INVALID {
            return 0;
        }
        let extra = match id {
            TIME_ZONE_ID_DAYLIGHT => info.daylight_bias,
            TIME_ZONE_ID_STANDARD => info.standard_bias,
            _ => 0, // TIME_ZONE_ID_UNKNOWN
        };
        -i64::from(info.bias + extra) * 60
    }
}

pub fn civil(days: i64) -> (i64, u32, u32) {
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

/// "2026-10-13 18:05" en hora local.
pub fn fmt_date(t: i64) -> String {
    let l = t + tz_offset();
    let (y, m, d) = civil(l.div_euclid(86400));
    let s = l.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", s / 3600, (s % 3600) / 60)
}

/// Fecha corta para listas: "18:05" si es hoy, "13 oct" este año, "2024-03-01" si no.
pub fn fmt_short(t: i64, now: i64) -> String {
    let off = tz_offset();
    let (l, n) = (t + off, now + off);
    let (y, m, d) = civil(l.div_euclid(86400));
    let (ny, _, _) = civil(n.div_euclid(86400));
    if l.div_euclid(86400) == n.div_euclid(86400) {
        let s = l.rem_euclid(86400);
        format!("{:02}:{:02}", s / 3600, (s % 3600) / 60)
    } else if y == ny {
        const MES: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"];
        format!("{d:>2} {}", MES[(m - 1) as usize])
    } else {
        format!("{y:04}-{m:02}-{d:02}")
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Recorta a `w` columnas con "…".
pub fn trunc(s: &str, w: usize) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut acc = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if acc + cw > w - 1 {
            break;
        }
        acc += cw;
        out.push(c);
    }
    out.push('…');
    out
}

/// Rellena con espacios a la derecha hasta `w` (recortando si sobra).
pub fn pad(s: &str, w: usize) -> String {
    let t = trunc(s, w);
    let n = w.saturating_sub(width(&t));
    format!("{t}{}", " ".repeat(n))
}

pub fn pad_left(s: &str, w: usize) -> String {
    let t = trunc(s, w);
    let n = w.saturating_sub(width(&t));
    format!("{}{t}", " ".repeat(n))
}

/// Parte un texto en líneas de ancho `w` (por palabras; palabras largas se cortan).
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(8);
    let mut out = Vec::new();
    for raw in s.lines() {
        if raw.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let indent: String = raw.chars().take_while(|c| *c == ' ').collect();
        let mut cur = String::new();
        for word in raw.split_whitespace() {
            let sep = if cur.is_empty() { indent.as_str() } else { " " };
            if !cur.is_empty() && width(&cur) + 1 + width(word) > w {
                out.push(std::mem::take(&mut cur));
                cur.push_str(&indent);
                cur.push_str("  ");
            } else {
                cur.push_str(sep);
            }
            let mut word = word.to_string();
            while width(&cur) + width(&word) > w && width(&word) > w / 2 {
                let room = w.saturating_sub(width(&cur)).max(1);
                let cut: String = word.chars().take(room).collect();
                let rest: String = word.chars().skip(room).collect();
                cur.push_str(&cut);
                out.push(std::mem::take(&mut cur));
                word = rest;
            }
            cur.push_str(&word);
        }
        out.push(cur);
    }
    out
}

/// Markdown mínimo → líneas con estilo (títulos, viñetas, **negrita**, `código`).
pub fn markdown(s: &str, w: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for l in wrap(s, w) {
        let t = l.trim_start();
        if let Some(h) = t.strip_prefix("### ").or_else(|| t.strip_prefix("## ")).or_else(|| t.strip_prefix("# ")) {
            lines.push(Line::from(Span::styled(
                h.replace("**", ""),
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        let (prefix, body) = if let Some(b) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            let ind = width(&l) - width(t);
            (format!("{}• ", " ".repeat(ind)), b.to_string())
        } else {
            (String::new(), l.clone())
        };
        let mut spans = vec![Span::styled(prefix, Style::new().fg(ACCENT))];
        spans.extend(inline_md(&body));
        lines.push(Line::from(spans));
    }
    lines
}

fn inline_md(s: &str) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut bold = false;
    let mut code = false;
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    let flush = |cur: &mut String, out: &mut Vec<Span<'static>>, bold: bool, code: bool| {
        if cur.is_empty() {
            return;
        }
        let mut st = Style::new();
        if bold {
            st = st.add_modifier(Modifier::BOLD).fg(Color::White);
        }
        if code {
            st = st.fg(HUNK);
        }
        out.push(Span::styled(std::mem::take(cur), st));
    };
    while let Some(c) = chars.next() {
        if c == '*' && chars.peek() == Some(&'*') {
            chars.next();
            flush(&mut cur, &mut out, bold, code);
            bold = !bold;
        } else if c == '`' {
            flush(&mut cur, &mut out, bold, code);
            code = !code;
        } else {
            cur.push(c);
        }
    }
    flush(&mut cur, &mut out, bold, code);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trunc_and_pad() {
        assert_eq!(trunc("abcdef", 4), "abc…");
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad_left("ab", 4), "  ab");
    }

    #[test]
    fn wrap_words() {
        let w = wrap("uno dos tres cuatro", 9);
        assert!(w.iter().all(|l| width(l) <= 9), "{w:?}");
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn civil_epoch() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20374), (2025, 10, 13));
    }

    #[test]
    fn tz_offset_en_minutos_y_rango_realista() {
        let off = tz_offset();
        assert_eq!(off % 60, 0, "el offset va en minutos exactos: {off}");
        assert!(off.abs() <= 14 * 3600, "offset fuera de rango: {off}");
    }
}
