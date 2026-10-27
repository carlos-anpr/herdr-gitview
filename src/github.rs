//! Pull Requests de GitHub vía gh; API bloqueante para los hilos de fondo.
use crate::{gitlab::parse_date, model::*};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::Value;
use std::{path::Path, process::Command};

const FIELDS: &str = "number,title,state,author,headRefName,baseRefName,createdAt,updatedAt,mergedAt,url,body,isDraft,headRefOid,mergeCommit,labels,comments,statusCheckRollup";

type Remote = (String, String, String);

/// Acepta HTTPS, SSH y sintaxis scp; nunca rutas locales ni subgrupos.
fn parse_remote(url: &str) -> Option<Remote> {
    let (authority, path) = if let Some((scheme, rest)) = url.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh") {
            return None;
        }
        rest.split_once('/')?
    } else {
        let (authority, path) = url.split_once(':')?;
        if !authority.contains('@') {
            return None;
        }
        (authority, path)
    };
    let host = authority
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .to_ascii_lowercase();
    let mut parts = path.trim_end_matches('/').split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    let name = name.strip_suffix(".git").unwrap_or(name);
    let valid = |s: &str| {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    if !valid(&host) || !valid(owner) || !valid(name) || parts.next().is_some() {
        return None;
    }
    Some((host, owner.to_owned(), name.to_owned()))
}

fn command(repo: &Path, program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(repo)
        .env("LC_ALL", "C")
        .env("GH_PROMPT_DISABLED", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn run(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = command(repo, "gh")
        .args(args)
        .output()
        .context("no se pudo ejecutar gh")?;
    if !output.status.success() {
        bail!(
            "gh {} falló ({}): {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn remote(repo: &Path) -> Result<Remote> {
    // Origin es el contrato de detección: no usar el fallback de otros remotos.
    let output = command(repo, "git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo)
        .output()
        .context("no se pudo consultar origin")?;
    if !output.status.success() {
        bail!(
            "no se pudo consultar origin: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_remote(String::from_utf8_lossy(&output.stdout).trim())
        .context("el remoto origin no tiene formato GitHub host/owner/repo")
}

pub fn is_github(repo: &Path) -> bool {
    let Ok((host, _, _)) = remote(repo) else {
        return false;
    };
    // gh valida tanto github.com como los hosts Enterprise que tiene configurados.
    run(repo, &["auth", "status", "--hostname", &host]).is_ok()
}

fn checks_status(checks: &Value) -> Option<String> {
    let checks = checks.as_array().filter(|checks| !checks.is_empty())?;
    let mut pending = false;
    for check in checks {
        let conclusion = check["conclusion"].as_str().unwrap_or_default();
        let state = check["state"].as_str().unwrap_or_default();
        if matches!(
            conclusion,
            "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "STALE"
        ) || matches!(state, "FAILURE" | "ERROR")
        {
            return Some("failed".into());
        }
        if !matches!(conclusion, "SUCCESS" | "NEUTRAL" | "SKIPPED") && state != "SUCCESS" {
            pending = true;
        }
    }
    Some(if pending { "running" } else { "success" }.into())
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct RawPr {
    number: u64,
    title: String,
    state: String,
    author: Value,
    head_ref_name: String,
    base_ref_name: String,
    created_at: String,
    updated_at: String,
    merged_at: Option<String>,
    url: String,
    body: String,
    is_draft: bool,
    head_ref_oid: Option<String>,
    merge_commit: Value,
    labels: Vec<Value>,
    comments: Vec<Value>,
    status_check_rollup: Value,
}

fn convert(raw: RawPr) -> Result<MergeRequest> {
    let state = if raw.merged_at.is_some() || raw.state == "MERGED" {
        "merged"
    } else if raw.state == "OPEN" {
        "opened"
    } else {
        "closed"
    };
    Ok(MergeRequest {
        iid: raw.number,
        title: raw.title,
        state: state.into(),
        author: raw.author["login"].as_str().unwrap_or_default().into(),
        source_branch: raw.head_ref_name,
        target_branch: raw.base_ref_name,
        created_at: parse_date(&raw.created_at)?,
        updated_at: parse_date(&raw.updated_at)?,
        merged_at: raw.merged_at.as_deref().map(parse_date).transpose()?,
        web_url: raw.url,
        description: raw.body,
        sha: raw.head_ref_oid,
        merge_commit_sha: raw.merge_commit["oid"].as_str().map(str::to_owned),
        squash_commit_sha: None,
        draft: raw.is_draft,
        pipeline_status: checks_status(&raw.status_check_rollup),
        labels: raw
            .labels
            .iter()
            .filter_map(|l| l["name"].as_str().map(str::to_owned))
            .collect(),
        user_notes_count: raw.comments.len() as u64,
    })
}

fn parse_list(data: &[u8], state: MrState, page: u32) -> Result<Vec<MergeRequest>> {
    if page == 0 {
        bail!("la página empieza en 1");
    }
    let rows = serde_json::from_slice::<Vec<RawPr>>(data)?
        .into_iter()
        .map(convert)
        .collect::<Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter(|mr| match state {
            MrState::Opened => mr.state == "opened",
            MrState::Merged => mr.state == "merged",
            MrState::Closed => mr.state == "closed",
            MrState::All => true,
        })
        .skip((page as usize - 1) * 50)
        .take(50)
        .collect())
}

pub fn list_mrs(repo: &Path, state: MrState, page: u32) -> Result<Vec<MergeRequest>> {
    if page == 0 {
        bail!("la página empieza en 1");
    }
    let (host, owner, name) = remote(repo)?;
    let project = format!("{host}/{owner}/{name}");
    let limit = u64::from(page) * 50;
    let state_arg = match state {
        MrState::Opened => "open",
        MrState::Merged => "merged",
        MrState::Closed => "closed",
        MrState::All => "all",
    };
    // gh closed también incluye merged. El filtro de búsqueda evita que consuman
    // plazas del límite y corten prematuramente la paginación de cerrados.
    let mut args = vec![
        "pr", "list", "-R", &project, "--state", state_arg, "--json", FIELDS,
    ];
    let limit = limit.to_string();
    args.extend(["--limit", &limit]);
    if state == MrState::Closed {
        args.extend(["--search", "is:unmerged"]);
    }
    parse_list(&run(repo, &args)?, state, page)
}

pub fn get_mr(repo: &Path, iid: u64) -> Result<MergeRequest> {
    let (host, owner, name) = remote(repo)?;
    let project = format!("{host}/{owner}/{name}");
    convert(serde_json::from_slice(&run(
        repo,
        &[
            "pr",
            "view",
            &iid.to_string(),
            "-R",
            &project,
            "--json",
            FIELDS,
        ],
    )?)?)
}

fn parse_pages<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<Vec<T>> {
    Ok(serde_json::from_slice::<Vec<Vec<T>>>(data)?
        .into_iter()
        .flatten()
        .collect())
}

fn api_pages<T: serde::de::DeserializeOwned>(
    repo: &Path,
    iid: u64,
    resource: &str,
) -> Result<Vec<T>> {
    let (host, owner, name) = remote(repo)?;
    let endpoint = format!("repos/{owner}/{name}/pulls/{iid}/{resource}?per_page=100");
    parse_pages(&run(
        repo,
        &[
            "api",
            "--hostname",
            &host,
            "--paginate",
            "--slurp",
            &endpoint,
        ],
    )?)
}

#[derive(Deserialize)]
struct RawFile {
    filename: String,
    previous_filename: Option<String>,
    status: String,
    patch: Option<String>,
    additions: u32,
    deletions: u32,
}

pub fn mr_diffs(repo: &Path, iid: u64) -> Result<Vec<MrFileDiff>> {
    Ok(api_pages::<RawFile>(repo, iid, "files")?
        .into_iter()
        .map(convert_file)
        .collect())
}

fn convert_file(file: RawFile) -> MrFileDiff {
    MrFileDiff {
        old_path: file
            .previous_filename
            .unwrap_or_else(|| file.filename.clone()),
        new_path: file.filename,
        new_file: file.status == "added",
        deleted_file: file.status == "removed",
        renamed_file: file.status == "renamed",
        diff: file.patch.unwrap_or_default(),
        additions: file.additions,
        deletions: file.deletions,
    }
}

pub fn mr_commits(repo: &Path, iid: u64) -> Result<Vec<MrCommit>> {
    api_pages::<Value>(repo, iid, "commits")?
        .into_iter()
        .map(|raw| {
            let id = raw["sha"].as_str().context("commit sin SHA")?.to_owned();
            Ok(MrCommit {
                short: id.chars().take(8).collect(),
                id,
                title: raw["commit"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .into(),
                author: raw["commit"]["author"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                time: parse_date(
                    raw["commit"]["author"]["date"]
                        .as_str()
                        .context("commit sin fecha")?,
                )?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_remote_forms_and_rejects_invalid_paths() {
        for url in [
            "https://github.com/owner/repo.git",
            "git@github.com:owner/repo",
            "ssh://git@github.com:2222/owner/repo.git",
            "https://user@github.com/owner/repo/",
        ] {
            assert_eq!(
                parse_remote(url),
                Some(("github.com".into(), "owner".into(), "repo".into()))
            );
        }
        for url in [
            "/local/repo",
            "https://github.com/owner",
            "git@github.com:owner/repo/extra",
            "https://github.com/owner/../",
            "file:///owner/repo",
        ] {
            assert_eq!(parse_remote(url), None, "{url}");
        }
    }

    #[test]
    fn summarizes_checks_with_failure_then_pending_precedence() {
        assert_eq!(checks_status(&json!([])), None);
        assert_eq!(
            checks_status(
                &json!([{"conclusion":"SUCCESS","status":"COMPLETED"},{"state":"SUCCESS"}])
            ),
            Some("success".into())
        );
        assert_eq!(
            checks_status(
                &json!([{"conclusion":"NEUTRAL","status":"COMPLETED"},{"conclusion":"SKIPPED","status":"COMPLETED"}])
            ),
            Some("success".into())
        );
        assert_eq!(
            checks_status(&json!([{"status":"IN_PROGRESS"},{"state":"SUCCESS"}])),
            Some("running".into())
        );
        assert_eq!(
            checks_status(
                &json!([{"status":"QUEUED"},{"conclusion":"FAILURE","status":"COMPLETED"}])
            ),
            Some("failed".into())
        );
        assert_eq!(
            checks_status(&json!([{"state":"ERROR"}])),
            Some("failed".into())
        );
    }

    #[test]
    fn normalizes_states_and_pages_after_filtering() {
        let rows = (1..=105).map(|n| json!({"number":n,"state":"CLOSED","mergedAt": if n == 1 {json!("2026-10-27T16:49:43Z")} else {json!(null)},"createdAt":"2026-10-27T16:49:43Z","updatedAt":"2026-10-27T16:49:43Z"})).collect::<Vec<_>>();
        let data = serde_json::to_vec(&rows).unwrap();
        let page = parse_list(&data, MrState::Closed, 2).unwrap();
        assert_eq!(page.len(), 50);
        assert_eq!(page[0].iid, 52);
        assert_eq!(page[49].iid, 101);
        assert!(page.iter().all(|mr| mr.state == "closed"));
        assert_eq!(
            parse_list(&data, MrState::Merged, 1).unwrap()[0].state,
            "merged"
        );
        assert_eq!(parse_list(&data, MrState::Closed, 3).unwrap().len(), 4);
        assert!(parse_list(&data, MrState::All, 0).is_err());
        let open = br#"[{"number":1,"state":"OPEN","createdAt":"2026-10-27T16:49:43Z","updatedAt":"2026-10-27T16:49:43Z"}]"#;
        assert_eq!(
            parse_list(open, MrState::Opened, 1).unwrap()[0].state,
            "opened"
        );
    }

    #[test]
    fn flattens_slurped_api_pages() {
        assert_eq!(
            parse_pages::<serde_json::Value>(br#"[[{"sha":"a"}],[{"sha":"b"}]]"#)
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    #[ignore = "requiere gh autenticado y HERDR_GITHUB_TEST_REPO"]
    fn reads_real_github_repo() {
        let repo = std::env::var_os("HERDR_GITHUB_TEST_REPO")
            .map(std::path::PathBuf::from)
            .expect("falta HERDR_GITHUB_TEST_REPO");
        assert!(is_github(&repo));
        for state in [
            MrState::Opened,
            MrState::Merged,
            MrState::Closed,
            MrState::All,
        ] {
            let rows = list_mrs(&repo, state, 1).unwrap();
            eprintln!("GitHub {state:?}: {} PRs", rows.len());
            assert!(rows.iter().all(|mr| match state {
                MrState::Opened => mr.state == "opened",
                MrState::Merged => mr.state == "merged",
                MrState::Closed => mr.state == "closed",
                MrState::All => true,
            }));
        }
        let prs = list_mrs(&repo, MrState::All, 1).unwrap();
        let pr = prs.first().expect("repo sin PRs");
        let detail = get_mr(&repo, pr.iid).unwrap();
        assert_eq!(detail.iid, pr.iid);
        let files = mr_diffs(&repo, pr.iid).unwrap();
        let commits = mr_commits(&repo, pr.iid).unwrap();
        assert!(!files.is_empty());
        assert!(!commits.is_empty());
        eprintln!(
            "GitHub #{}: {} files, {} commits",
            pr.iid,
            files.len(),
            commits.len()
        );
    }

    #[test]
    fn converts_renames_and_files_without_patch() {
        let pages = br#"[[{"filename":"new.rs","previous_filename":"old.rs","status":"renamed","patch":"@@ -1 +1 @@\n-a\n+b","additions":1,"deletions":1}],[{"filename":"binary.bin","status":"added","additions":0,"deletions":0},{"filename":"gone.rs","status":"removed","additions":0,"deletions":3}]]"#;
        let files = parse_pages::<RawFile>(pages)
            .unwrap()
            .into_iter()
            .map(convert_file)
            .collect::<Vec<_>>();
        assert_eq!(files[0].old_path, "old.rs");
        assert!(files[0].renamed_file);
        assert!(files[0].diff.starts_with("@@"));
        assert_eq!((files[0].additions, files[0].deletions), (1, 1));
        assert!(files[1].new_file);
        assert!(files[1].diff.is_empty());
        assert!(files[2].deleted_file);
    }
}
