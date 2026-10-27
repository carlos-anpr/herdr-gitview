//! Backend git vía CLI `git` (subprocesos). PROPIETARIO: Codex.
//! Todas las funciones son bloqueantes; el UI las llama desde hilos de fondo.

use crate::model::*;
use anyhow::{Context, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::Output;

const FIELD: char = '\x1f';
const RECORD: char = '\x1e';
const LOG_FORMAT: &str = "%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D%x1e";

fn git_output(repo: &Path, args: &[&str]) -> Result<Output> {
    crate::platform::cmd("git")
        .arg("-C")
        .arg(repo)
        .arg("--no-pager")
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .with_context(|| {
            format!(
                "no se pudo ejecutar git -C {} {}",
                repo.display(),
                args.join(" ")
            )
        })
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_output(repo, args)?;
    if !output.status.success() {
        bail!(
            "git -C {} {} falló ({}): {}",
            repo.display(),
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn git_text(repo: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git(repo, args)?).into_owned())
}

pub fn repo_root(path: &Path) -> Option<PathBuf> {
    let base = if path.is_dir() { path } else { path.parent()? };
    Some(PathBuf::from(
        git_text(base, &["rev-parse", "--show-toplevel"])
            .ok()?
            .trim(),
    ))
}

pub fn head_branch(repo: &Path) -> Result<String> {
    match git_text(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        Ok(name) => Ok(name.trim().to_owned()),
        Err(_) => Ok(format!(
            "HEAD detached at {}",
            git_text(repo, &["rev-parse", "--short", "HEAD"])?.trim()
        )),
    }
}

pub fn head_sha(repo: &Path) -> Option<String> {
    git_text(repo, &["rev-parse", "--verify", "HEAD"])
        .ok()
        .map(|value| value.trim().to_owned())
}

pub fn remote_url(repo: &Path) -> Option<String> {
    if let Ok(url) = git_text(repo, &["remote", "get-url", "origin"]) {
        return Some(url.trim().to_owned());
    }
    let remotes = git_text(repo, &["remote"]).ok()?;
    git_text(repo, &["remote", "get-url", remotes.lines().next()?])
        .ok()
        .map(|value| value.trim().to_owned())
}

fn parse_refs(value: &str) -> Vec<RefLabel> {
    let mut refs = Vec::new();
    for raw in value.split(", ") {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        if let Some(name) = raw.strip_prefix("HEAD -> ") {
            let name = name.strip_prefix("refs/heads/").unwrap_or(name);
            refs.push(RefLabel {
                name: name.to_owned(),
                kind: RefKind::Head,
            });
            refs.push(RefLabel {
                name: name.to_owned(),
                kind: RefKind::LocalBranch,
            });
            continue;
        }
        if raw == "HEAD" {
            refs.push(RefLabel {
                name: raw.to_owned(),
                kind: RefKind::Head,
            });
            continue;
        }
        if let Some(name) = raw.strip_prefix("refs/heads/") {
            refs.push(RefLabel {
                name: name.to_owned(),
                kind: RefKind::LocalBranch,
            });
            continue;
        }
        if let Some(name) = raw.strip_prefix("refs/remotes/") {
            if name.ends_with("/HEAD") || name.contains("/HEAD -> ") {
                continue;
            }
            refs.push(RefLabel {
                name: name.to_owned(),
                kind: RefKind::RemoteBranch,
            });
            continue;
        }
        if let Some(name) = raw
            .strip_prefix("tag: refs/tags/")
            .or_else(|| raw.strip_prefix("refs/tags/"))
        {
            refs.push(RefLabel {
                name: name.to_owned(),
                kind: RefKind::Tag,
            });
        }
    }
    refs
}

fn parse_log(text: &str) -> Result<Vec<Commit>> {
    text.split(RECORD)
        .filter(|record| !record.trim_matches('\n').is_empty())
        .map(|record| {
            let fields: Vec<&str> = record.trim_start_matches('\n').split(FIELD).collect();
            if fields.len() != 8 {
                bail!("registro de git log mal formado: {} campos", fields.len());
            }
            Ok(Commit {
                id: fields[0].to_owned(),
                short: fields[1].to_owned(),
                parents: fields[2].split_whitespace().map(str::to_owned).collect(),
                author: fields[3].to_owned(),
                email: fields[4].to_owned(),
                time: fields[5].parse().context("fecha inválida en git log")?,
                subject: fields[6].to_owned(),
                refs: parse_refs(fields[7]),
            })
        })
        .collect()
}

pub fn log(repo: &Path, skip: usize, limit: usize) -> Result<Vec<Commit>> {
    let skip_arg = format!("--skip={skip}");
    let limit_arg = format!("-n{limit}");
    let format_arg = format!("--format={LOG_FORMAT}");
    parse_log(&git_text(
        repo,
        &[
            "log",
            "--all",
            "--topo-order",
            "--date-order",
            "--decorate=full",
            &format_arg,
            &skip_arg,
            &limit_arg,
        ],
    )?)
}

pub fn commit_detail(repo: &Path, sha: &str) -> Result<CommitDetail> {
    let format = format!("--format={LOG_FORMAT}%x1f%B%x1f%cn%x1f%ct");
    let text = git_text(
        repo,
        &["show", "--no-patch", "--decorate=full", &format, sha],
    )?;
    let (head, tail) = text
        .split_once(RECORD)
        .ok_or_else(|| anyhow!("detalle de commit mal formado"))?;
    let commit = parse_log(&format!("{head}{RECORD}"))?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("commit vacío"))?;
    let mut fields = tail.strip_prefix(FIELD).unwrap_or(tail).rsplitn(3, FIELD);
    let commit_time = fields
        .next()
        .ok_or_else(|| anyhow!("falta fecha de committer"))?
        .trim()
        .parse()?;
    let committer = fields
        .next()
        .ok_or_else(|| anyhow!("falta committer"))?
        .to_owned();
    let message = fields
        .next()
        .ok_or_else(|| anyhow!("falta mensaje"))?
        .trim_end_matches('\n')
        .to_owned();
    Ok(CommitDetail {
        commit,
        message,
        committer,
        commit_time,
    })
}

#[derive(Debug, PartialEq, Eq)]
struct NumStat {
    path: String,
    old_path: Option<String>,
    additions: u32,
    deletions: u32,
    binary: bool,
}

fn parse_numstat(data: &[u8]) -> Result<Vec<NumStat>> {
    let fields: Vec<&[u8]> = data.split(|byte| *byte == 0).collect();
    let mut result = Vec::new();
    let mut index = 0;
    while index < fields.len() && !fields[index].is_empty() {
        let header = String::from_utf8_lossy(fields[index]);
        let mut parts = header.splitn(3, '\t');
        let adds = parts
            .next()
            .ok_or_else(|| anyhow!("numstat sin adiciones"))?;
        let dels = parts
            .next()
            .ok_or_else(|| anyhow!("numstat sin borrados"))?;
        let path_part = parts.next().ok_or_else(|| anyhow!("numstat sin ruta"))?;
        let binary = adds == "-" || dels == "-";
        let count = |value: &str| -> Result<u32> {
            if value == "-" {
                Ok(0)
            } else {
                Ok(value.parse()?)
            }
        };
        if path_part.is_empty() {
            let old = fields
                .get(index + 1)
                .ok_or_else(|| anyhow!("rename sin ruta anterior"))?;
            let new = fields
                .get(index + 2)
                .ok_or_else(|| anyhow!("rename sin ruta nueva"))?;
            result.push(NumStat {
                path: String::from_utf8_lossy(new).into_owned(),
                old_path: Some(String::from_utf8_lossy(old).into_owned()),
                additions: count(adds)?,
                deletions: count(dels)?,
                binary,
            });
            index += 3;
        } else {
            result.push(NumStat {
                path: path_part.to_owned(),
                old_path: None,
                additions: count(adds)?,
                deletions: count(dels)?,
                binary,
            });
            index += 1;
        }
    }
    Ok(result)
}

fn parse_name_status(data: &[u8]) -> Result<Vec<(char, String, Option<String>)>> {
    let fields: Vec<&[u8]> = data.split(|byte| *byte == 0).collect();
    let mut result = Vec::new();
    let mut index = 0;
    while index < fields.len() && !fields[index].is_empty() {
        let status = String::from_utf8_lossy(fields[index])
            .chars()
            .next()
            .ok_or_else(|| anyhow!("status vacío"))?;
        if matches!(status, 'R' | 'C') {
            let old = fields
                .get(index + 1)
                .ok_or_else(|| anyhow!("rename sin ruta anterior"))?;
            let new = fields
                .get(index + 2)
                .ok_or_else(|| anyhow!("rename sin ruta nueva"))?;
            result.push((
                status,
                String::from_utf8_lossy(new).into_owned(),
                Some(String::from_utf8_lossy(old).into_owned()),
            ));
            index += 3;
        } else {
            let path = fields
                .get(index + 1)
                .ok_or_else(|| anyhow!("status sin ruta"))?;
            result.push((status, String::from_utf8_lossy(path).into_owned(), None));
            index += 2;
        }
    }
    Ok(result)
}

pub fn commit_files(repo: &Path, sha: &str) -> Result<Vec<FileChange>> {
    let parent = first_parent(repo, sha)?;
    let mut num_args = if let Some(parent) = parent.as_deref() {
        vec!["diff", "-r", "-M", "-z", "--numstat", parent, sha]
    } else {
        vec![
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "-M",
            "-z",
            "--numstat",
            sha,
        ]
    };
    let stats = parse_numstat(&git(repo, &num_args)?)?;
    let status_flag = num_args
        .iter()
        .position(|arg| *arg == "--numstat")
        .expect("los argumentos incluyen numstat");
    num_args[status_flag] = "--name-status";
    let status_args = num_args;
    parse_name_status(&git(repo, &status_args)?)?
        .into_iter()
        .map(|(status, path, old)| {
            let stat = stats
                .iter()
                .find(|entry| entry.path == path)
                .ok_or_else(|| anyhow!("no hay numstat para {path}"))?;
            Ok(FileChange {
                path,
                old_path: old.or_else(|| stat.old_path.clone()),
                status,
                additions: stat.additions,
                deletions: stat.deletions,
                binary: stat.binary,
            })
        })
        .collect()
}

fn first_parent(repo: &Path, sha: &str) -> Result<Option<String>> {
    let line = git_text(repo, &["rev-list", "--parents", "-n1", sha])?;
    Ok(line.split_whitespace().nth(1).map(str::to_owned))
}

fn show_diff(repo: &Path, sha: &str, path: Option<&str>) -> Result<String> {
    let parent = first_parent(repo, sha)?;
    let mut args = if let Some(parent) = parent.as_deref() {
        vec!["diff", "--no-color", "-M", parent, sha]
    } else {
        vec!["show", "--format=", "--no-color", "--root", "-M", sha]
    };
    if let Some(path) = path {
        args.extend(["--", path]);
    }
    git_text(repo, &args)
}

pub fn file_diff(repo: &Path, sha: &str, path: &str) -> Result<String> {
    show_diff(repo, sha, Some(path))
}

pub fn commit_diff(repo: &Path, sha: &str, max_bytes: usize) -> Result<String> {
    let diff = show_diff(repo, sha, None)?;
    if diff.len() <= max_bytes {
        return Ok(diff);
    }
    let mut end = max_bytes.min(diff.len());
    while end > 0 && !diff.is_char_boundary(end) {
        end -= 1;
    }
    Ok(format!("{}\n[...truncado]", &diff[..end]))
}

fn parse_status(data: &[u8]) -> Result<Vec<StatusEntry>> {
    let fields: Vec<&[u8]> = data.split(|byte| *byte == 0).collect();
    let mut result = Vec::new();
    let mut index = 0;
    while index < fields.len() && !fields[index].is_empty() {
        let entry = fields[index];
        if entry.len() < 3 || entry[2] != b' ' {
            bail!("entrada de status mal formada");
        }
        let index_status = entry[0] as char;
        let worktree = entry[1] as char;
        let path = String::from_utf8_lossy(&entry[3..]).into_owned();
        if matches!(index_status, 'R' | 'C') || matches!(worktree, 'R' | 'C') {
            let old = fields
                .get(index + 1)
                .ok_or_else(|| anyhow!("rename de status sin ruta anterior"))?;
            result.push(StatusEntry {
                path,
                old_path: Some(String::from_utf8_lossy(old).into_owned()),
                index: index_status,
                worktree,
            });
            index += 2;
        } else {
            result.push(StatusEntry {
                path,
                old_path: None,
                index: index_status,
                worktree,
            });
            index += 1;
        }
    }
    Ok(result)
}

pub fn status(repo: &Path) -> Result<Vec<StatusEntry>> {
    parse_status(&git(
        repo,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?)
}

pub fn working_diff(repo: &Path, path: &str, staged: bool) -> Result<String> {
    let untracked = status(repo)?
        .iter()
        .any(|entry| entry.path == path && entry.index == '?' && entry.worktree == '?');
    if untracked {
        let output = git_output(
            repo,
            &["diff", "--no-index", "--no-color", "--", "/dev/null", path],
        )?;
        if output.status.success() || output.status.code() == Some(1) {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        bail!(
            "git diff --no-index falló: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut args = vec!["diff", "--no-color", "-M"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", path]);
    git_text(repo, &args)
}

pub fn file_log(repo: &Path, path: &str, limit: usize) -> Result<Vec<Commit>> {
    let limit_arg = format!("-n{limit}");
    let format_arg = format!("--format={LOG_FORMAT}");
    parse_log(&git_text(
        repo,
        &[
            "log",
            "--follow",
            "--date-order",
            "--decorate=full",
            &format_arg,
            &limit_arg,
            "--",
            path,
        ],
    )?)
}

pub fn file_at(repo: &Path, sha: &str, path: &str) -> Result<String> {
    let data = git(repo, &["show", &format!("{sha}:{path}")])?;
    if data.iter().take(8192).any(|byte| *byte == 0) {
        bail!("{path} es binario");
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_repo() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("herdr-git-test-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        let run = |args: &[&str]| assert!(git_output(&path, args).unwrap().status.success());
        run(&["init", "-q"]);
        run(&["config", "user.name", "Test User"]);
        run(&["config", "user.email", "test@example.com"]);
        path
    }

    #[test]
    fn parses_log_and_refs() {
        let text = "abc\x1fabc1234\x1fp1 p2\x1fAna\x1fa@b.c\x1f1700000000\x1fSubject\x1fHEAD -> refs/heads/main, refs/remotes/origin/main, refs/remotes/origin/HEAD, tag: refs/tags/v1\x1e";
        let commits = parse_log(text).unwrap();
        assert_eq!(commits[0].parents, ["p1", "p2"]);
        assert_eq!(commits[0].refs.len(), 4);
        assert_eq!(commits[0].refs[0].kind, RefKind::Head);
        assert_eq!(commits[0].refs[0].name, "main");
        assert_eq!(commits[0].refs[1].kind, RefKind::LocalBranch);
        assert_eq!(commits[0].refs[1].name, "main");
        assert_eq!(commits[0].refs[3].name, "v1");
    }

    #[test]
    fn parses_numstat_and_status() {
        let stats = parse_numstat(
            b"2\t1\tfile.txt\0-\t-\tbin.dat\0"
                .iter()
                .copied()
                .chain(b"5\t0\t\0old.rs\0new.rs\0".iter().copied())
                .collect::<Vec<_>>()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(stats[0].additions, 2);
        assert!(stats[1].binary);
        assert_eq!(stats[2].old_path.as_deref(), Some("old.rs"));
        let entries = parse_status(b" M file.txt\0R  new.rs\0old.rs\0?? new.txt\0").unwrap();
        assert_eq!(entries[1].old_path.as_deref(), Some("old.rs"));
        assert_eq!(entries[2].worktree, '?');
    }

    #[test]
    fn accepts_non_utf8_commit_messages() {
        let repo = temp_repo();
        git(&repo, &["config", "i18n.commitEncoding", "ISO-8859-1"]).unwrap();
        git(&repo, &["config", "i18n.logOutputEncoding", "ISO-8859-1"]).unwrap();
        fs::write(repo.join("one.txt"), "one\n").unwrap();
        fs::write(repo.join("message.txt"), b"cambio en espa\xf1ol\n").unwrap();
        git(&repo, &["add", "one.txt"]).unwrap();
        git(
            &repo,
            &[
                "commit",
                "-q",
                "-F",
                repo.join("message.txt").to_str().unwrap(),
            ],
        )
        .unwrap();
        let commits = log(&repo, 0, 1).unwrap();
        assert_eq!(commits.len(), 1);
        assert!(commits[0].subject.contains('�'));
        fs::remove_dir_all(repo).unwrap();
    }

    #[test]
    fn reads_a_temporary_repository() {
        let repo = temp_repo();
        fs::write(repo.join("one.txt"), "one\n").unwrap();
        git(&repo, &["add", "."]).unwrap();
        git(&repo, &["commit", "-q", "-m", "first"]).unwrap();
        let initial_branch = head_branch(&repo).unwrap();
        let commits = log(&repo, 0, 10).unwrap();
        assert_eq!(commits.len(), 1);
        let files = commit_files(&repo, &commits[0].id).unwrap();
        assert_eq!(files[0].status, 'A');
        assert_eq!(file_at(&repo, &commits[0].id, "one.txt").unwrap(), "one\n");
        fs::write(repo.join("new.txt"), "new\n").unwrap();
        assert!(
            working_diff(&repo, "new.txt", false)
                .unwrap()
                .contains("+new")
        );
        fs::remove_file(repo.join("new.txt")).unwrap();

        git(&repo, &["checkout", "-q", "-b", "feature"]).unwrap();
        fs::write(repo.join("feature.txt"), "feature\n").unwrap();
        git(&repo, &["add", "."]).unwrap();
        git(&repo, &["commit", "-q", "-m", "feature"]).unwrap();
        git(&repo, &["checkout", "-q", &initial_branch]).unwrap();
        fs::write(repo.join("main.txt"), "main\n").unwrap();
        git(&repo, &["add", "."]).unwrap();
        git(&repo, &["commit", "-q", "-m", "main"]).unwrap();
        git(&repo, &["merge", "-q", "--no-ff", "feature", "-m", "merge"]).unwrap();
        let merge = head_sha(&repo).unwrap();
        let merge_files = commit_files(&repo, &merge).unwrap();
        assert_eq!(merge_files.len(), 1);
        assert_eq!(merge_files[0].path, "feature.txt");
        assert!(
            commit_diff(&repo, &merge, usize::MAX)
                .unwrap()
                .contains("+feature")
        );
        fs::remove_dir_all(repo).unwrap();
    }
}
