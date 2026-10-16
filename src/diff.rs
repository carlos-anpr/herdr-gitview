//! Diff unificado → líneas con número antiguo/nuevo para pintar.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Meta,
    Hunk,
    Ctx,
    Add,
    Del,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: Kind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct DiffView {
    pub title: String,
    pub lines: Vec<DiffLine>,
    pub scroll: usize,
    pub adds: u32,
    pub dels: u32,
}

impl DiffView {
    pub fn new(title: String, raw: &str) -> Self {
        let lines = parse(raw);
        let adds = lines.iter().filter(|l| l.kind == Kind::Add).count() as u32;
        let dels = lines.iter().filter(|l| l.kind == Kind::Del).count() as u32;
        DiffView { title, lines, scroll: 0, adds, dels }
    }

    /// Índices de las cabeceras de hunk (para saltar con n/p).
    pub fn hunks(&self) -> impl Iterator<Item = usize> + '_ {
        self.lines.iter().enumerate().filter(|(_, l)| l.kind == Kind::Hunk).map(|(i, _)| i)
    }
}

fn hunk_start(h: &str) -> (u32, u32) {
    // @@ -a,b +c,d @@
    let mut old = 0;
    let mut new = 0;
    for part in h.split_whitespace().skip(1).take(2) {
        let n = part[1..].split(',').next().and_then(|x| x.parse().ok()).unwrap_or(0);
        if part.starts_with('-') {
            old = n;
        } else if part.starts_with('+') {
            new = n;
        }
    }
    (old, new)
}

pub fn parse(raw: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let (mut o, mut n) = (0u32, 0u32);
    let mut in_hunk = false;
    for l in raw.lines() {
        let l = l.replace('\t', "    ");
        if l.starts_with("@@") {
            (o, n) = hunk_start(&l);
            in_hunk = true;
            out.push(DiffLine { kind: Kind::Hunk, old: None, new: None, text: l });
        } else if !in_hunk
            || l.starts_with("diff --git")
            || l.starts_with("index ")
            || l.starts_with("--- ")
            || l.starts_with("+++ ")
        {
            if l.starts_with("diff --git") {
                in_hunk = false;
            }
            out.push(DiffLine { kind: Kind::Meta, old: None, new: None, text: l });
        } else if let Some(t) = l.strip_prefix('+') {
            out.push(DiffLine { kind: Kind::Add, old: None, new: Some(n), text: t.to_string() });
            n += 1;
        } else if let Some(t) = l.strip_prefix('-') {
            out.push(DiffLine { kind: Kind::Del, old: Some(o), new: None, text: t.to_string() });
            o += 1;
        } else if l.starts_with('\\') {
            out.push(DiffLine { kind: Kind::Meta, old: None, new: None, text: l });
        } else {
            let t = l.strip_prefix(' ').unwrap_or(&l).to_string();
            out.push(DiffLine { kind: Kind::Ctx, old: Some(o), new: Some(n), text: t });
            o += 1;
            n += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_lines() {
        let d = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -10,3 +10,3 @@ fn\n a\n-b\n+c\n d\n";
        let v = parse(d);
        assert_eq!(v[3].kind, Kind::Hunk);
        assert_eq!((v[4].old, v[4].new), (Some(10), Some(10)));
        assert_eq!((v[5].kind, v[5].old), (Kind::Del, Some(11)));
        assert_eq!((v[6].kind, v[6].new), (Kind::Add, Some(11)));
        assert_eq!((v[7].old, v[7].new), (Some(12), Some(12)));
    }

    #[test]
    fn gitlab_hunks_without_header() {
        let v = parse("@@ -1 +1 @@\n-a\n+b\n");
        assert_eq!(v.iter().filter(|l| l.kind == Kind::Add).count(), 1);
    }
}
