//! Merge Requests de GitLab vía `glab api` (reutiliza la auth de glab). PROPIETARIO: Codex.
//! Bloqueante; el UI llama desde hilos de fondo.

use crate::git;
use crate::model::*;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn remote_host(url: &str) -> Option<&str> {
    if let Some(rest) = url.split_once("://").map(|(_, rest)| rest) {
        let authority = rest
            .split(['/', ':'])
            .next()
            .filter(|host| !host.is_empty())?;
        return Some(
            authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host),
        );
    }
    let (_, rest) = url.split_once('@')?;
    rest.split(':').next().filter(|host| !host.is_empty())
}

fn glab_configs() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
        paths.push(PathBuf::from(base).join("glab-cli/config.yml"));
    }
    if let Some(base) = std::env::var_os("APPDATA") {
        paths.push(PathBuf::from(base).join("glab-cli/config.yml"));
    }
    for home in ["HOME", "USERPROFILE"] {
        if let Some(base) = std::env::var_os(home) {
            paths.push(PathBuf::from(base).join(".config/glab-cli/config.yml"));
        }
    }
    paths
}

fn configured_hosts(config: &str) -> Vec<&str> {
    let mut in_hosts = false;
    let mut hosts = Vec::new();
    for line in config.lines() {
        if line.trim() == "hosts:" {
            in_hosts = true;
            continue;
        }
        if !in_hosts || line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            break;
        }
        if indent <= 4
            && let Some(host) = line.trim().strip_suffix(':')
        {
            hosts.push(host.trim_matches(['\'', '"']));
        }
    }
    hosts
}

/// true si el remoto del repo apunta a un host GitLab con glab autenticado.
pub fn is_gitlab(repo: &Path) -> bool {
    let Some(url) = git::remote_url(repo) else {
        return false;
    };
    let Some(host) = remote_host(&url) else {
        return false;
    };
    glab_configs().iter().filter_map(|path| fs::read_to_string(path).ok())
        .any(|config| configured_hosts(&config).contains(&host))
}

fn api(repo: &Path, endpoint: &str) -> Result<Vec<u8>> {
    let mut command = Command::new("glab");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .args(["api", endpoint])
        .current_dir(repo)
        .env("LC_ALL", "C")
        .output()
        .with_context(|| format!("no se pudo ejecutar glab api {endpoint}"))?;
    if !output.status.success() {
        bail!(
            "glab api {endpoint} falló ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

pub(crate) fn parse_date(value: &str) -> Result<i64> {
    if value.len() < 20 {
        bail!("fecha ISO8601 inválida: {value}");
    }
    let number = |range: std::ops::Range<usize>| -> Result<i64> {
        Ok(value
            .get(range)
            .context("fecha ISO8601 incompleta")?
            .parse()?)
    };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    let days = days_from_civil(year, month, day);
    let mut seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    let suffix = &value[19..];
    let zone = if suffix.starts_with('.') {
        suffix
            .find(['Z', '+', '-'])
            .map(|index| &suffix[index..])
            .context("fecha ISO8601 sin zona")?
    } else {
        suffix
    };
    if zone != "Z" {
        if zone.len() < 6 || &zone[3..4] != ":" {
            bail!("zona ISO8601 inválida: {zone}");
        }
        let offset_hours: i64 = zone[1..3].parse()?;
        let offset_minutes: i64 = zone[4..6].parse()?;
        let offset = offset_hours * 3_600 + offset_minutes * 60;
        match &zone[..1] {
            "+" => seconds -= offset,
            "-" => seconds += offset,
            _ => bail!("zona ISO8601 inválida: {zone}"),
        }
    }
    Ok(seconds)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[derive(Deserialize, Default)]
struct RawUser {
    #[serde(default)]
    username: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize, Default)]
struct RawPipeline {
    status: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawMr {
    iid: u64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    author: RawUser,
    #[serde(default)]
    source_branch: String,
    #[serde(default)]
    target_branch: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    merged_at: Option<String>,
    #[serde(default)]
    web_url: String,
    #[serde(default)]
    description: String,
    sha: Option<String>,
    merge_commit_sha: Option<String>,
    squash_commit_sha: Option<String>,
    draft: Option<bool>,
    work_in_progress: Option<bool>,
    head_pipeline: Option<RawPipeline>,
    pipeline: Option<RawPipeline>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    user_notes_count: u64,
}

fn convert_mr(raw: RawMr) -> Result<MergeRequest> {
    let pipeline_status = raw
        .head_pipeline
        .and_then(|pipeline| pipeline.status)
        .or_else(|| raw.pipeline.and_then(|pipeline| pipeline.status));
    Ok(MergeRequest {
        iid: raw.iid,
        title: raw.title,
        state: raw.state,
        author: if raw.author.username.is_empty() {
            raw.author.name
        } else {
            raw.author.username
        },
        source_branch: raw.source_branch,
        target_branch: raw.target_branch,
        created_at: parse_date(&raw.created_at)?,
        updated_at: parse_date(&raw.updated_at)?,
        merged_at: raw.merged_at.as_deref().map(parse_date).transpose()?,
        web_url: raw.web_url,
        description: raw.description,
        sha: raw.sha,
        merge_commit_sha: raw.merge_commit_sha,
        squash_commit_sha: raw.squash_commit_sha,
        draft: raw.draft.or(raw.work_in_progress).unwrap_or(false),
        pipeline_status,
        labels: raw.labels,
        user_notes_count: raw.user_notes_count,
    })
}

fn parse_mrs(data: &[u8]) -> Result<Vec<MergeRequest>> {
    serde_json::from_slice::<Vec<RawMr>>(data)?
        .into_iter()
        .map(convert_mr)
        .collect()
}

/// MRs del proyecto del repo, más recientes primero (`order_by=updated_at`), 50 por página (page empieza en 1).
pub fn list_mrs(repo: &Path, state: MrState, page: u32) -> Result<Vec<MergeRequest>> {
    let state = match state {
        MrState::Opened => "opened",
        MrState::Merged => "merged",
        MrState::Closed => "closed",
        MrState::All => "all",
    };
    let endpoint = format!(
        "projects/:id/merge_requests?state={state}&order_by=updated_at&sort=desc&per_page=50&page={page}&with_labels_details=false"
    );
    parse_mrs(&api(repo, &endpoint)?)
}

/// Detalle de un MR (incluye pipeline si está).
pub fn get_mr(repo: &Path, iid: u64) -> Result<MergeRequest> {
    let raw: RawMr =
        serde_json::from_slice(&api(repo, &format!("projects/:id/merge_requests/{iid}"))?)?;
    convert_mr(raw)
}

#[derive(Deserialize, Default)]
struct RawDiff {
    #[serde(default)]
    old_path: String,
    #[serde(default)]
    new_path: String,
    #[serde(default)]
    new_file: bool,
    #[serde(default)]
    deleted_file: bool,
    #[serde(default)]
    renamed_file: bool,
    #[serde(default)]
    diff: String,
}

fn convert_diff(raw: RawDiff) -> MrFileDiff {
    let (mut additions, mut deletions) = (0, 0);
    for line in raw.diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }
    MrFileDiff {
        old_path: raw.old_path,
        new_path: raw.new_path,
        new_file: raw.new_file,
        deleted_file: raw.deleted_file,
        renamed_file: raw.renamed_file,
        diff: raw.diff,
        additions,
        deletions,
    }
}

#[derive(Deserialize)]
struct RawChanges {
    changes: Vec<RawDiff>,
}

/// Ficheros y diffs del MR (`/merge_requests/:iid/diffs`, paginado hasta el final).
pub fn mr_diffs(repo: &Path, iid: u64) -> Result<Vec<MrFileDiff>> {
    let mut all = Vec::new();
    for page in 1.. {
        let endpoint = format!("projects/:id/merge_requests/{iid}/diffs?per_page=100&page={page}");
        let data = match api(repo, &endpoint) {
            Ok(data) => data,
            Err(error) if page == 1 => {
                let fallback = api(repo, &format!("projects/:id/merge_requests/{iid}/changes"))
                    .with_context(|| {
                        format!("el endpoint de diffs tampoco está disponible: {error:#}")
                    })?;
                let changes: RawChanges = serde_json::from_slice(&fallback)?;
                return Ok(changes.changes.into_iter().map(convert_diff).collect());
            }
            Err(error) => return Err(error),
        };
        let page_diffs: Vec<RawDiff> = serde_json::from_slice(&data)?;
        let count = page_diffs.len();
        all.extend(page_diffs.into_iter().map(convert_diff));
        if count < 100 {
            break;
        }
    }
    Ok(all)
}

#[derive(Deserialize, Default)]
struct RawCommit {
    #[serde(default)]
    id: String,
    #[serde(default)]
    short_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    author_name: String,
    #[serde(default)]
    authored_date: String,
    #[serde(default)]
    created_at: String,
}

/// Commits del MR.
pub fn mr_commits(repo: &Path, iid: u64) -> Result<Vec<MrCommit>> {
    let raw: Vec<RawCommit> = serde_json::from_slice(&api(
        repo,
        &format!("projects/:id/merge_requests/{iid}/commits?per_page=100"),
    )?)?;
    raw.into_iter()
        .map(|commit| {
            let date = if commit.authored_date.is_empty() {
                &commit.created_at
            } else {
                &commit.authored_date
            };
            Ok(MrCommit {
                id: commit.id,
                short: commit.short_id,
                title: commit.title,
                author: commit.author_name,
                time: parse_date(date)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hosts_and_remote_urls() {
        let config = "hosts:\n    gitlab.example.com:\n        token: secret\n    gitlab.com:\n        token: secret\n";
        assert_eq!(configured_hosts(config), ["gitlab.example.com", "gitlab.com"]);
        assert_eq!(
            remote_host("git@gitlab.example.com:group/repo.git"),
            Some("gitlab.example.com")
        );
        assert_eq!(
            remote_host("https://gitlab.com/group/repo.git"),
            Some("gitlab.com")
        );
        assert_eq!(
            remote_host("ssh://git@gitlab.example.com/group/repo.git"),
            Some("gitlab.example.com")
        );
    }

    #[test]
    fn parses_iso_dates_with_zones() {
        assert_eq!(parse_date("1970-01-01T00:00:00Z").unwrap(), 0);
        assert_eq!(parse_date("1970-01-01T01:00:00+01:00").unwrap(), 0);
        assert_eq!(parse_date("1969-12-31T19:00:00-05:00").unwrap(), 0);
        assert_eq!(
            parse_date("2024-01-02T03:04:05.123Z").unwrap(),
            1_704_164_645
        );
    }

    #[test]
    fn parses_real_mr_json_with_draft_and_work_in_progress() {
        let fixture = br#"[{"iid":105,"state":"merged","source_branch":"fix/mail-auth-method-case","target_branch":"develop","created_at":"2026-10-06T08:52:23.719+02:00","updated_at":"2026-10-06T09:06:54.210+02:00","merged_at":"2026-10-06T09:06:46.248+02:00","draft":false,"work_in_progress":false,"sha":"d19de20cf8118f520590b1a3f2ed7f3bd10d5e8c","merge_commit_sha":"9c31f32f7f273dc160000f4c21c5defe1da0831a","squash_commit_sha":null,"head_pipeline":null,"pipeline":null,"labels":[],"user_notes_count":0}]"#;
        let mrs = parse_mrs(fixture).unwrap();
        assert_eq!(mrs[0].iid, 105);
        assert!(!mrs[0].draft);
        assert_eq!(
            mrs[0].merge_commit_sha.as_deref(),
            Some("9c31f32f7f273dc160000f4c21c5defe1da0831a")
        );
        assert_eq!(mrs[0].squash_commit_sha, None);
    }

    #[test]
    fn head_pipeline_takes_precedence_over_pipeline() {
        let fixture = br#"[{"iid":7,"created_at":"2024-01-02T03:04:05Z","updated_at":"2024-01-02T03:04:05Z","head_pipeline":{"status":"success"},"pipeline":{"status":"failed"}}]"#;
        assert_eq!(
            parse_mrs(fixture).unwrap()[0].pipeline_status.as_deref(),
            Some("success")
        );
    }

    #[test]
    fn draft_takes_precedence_over_legacy_work_in_progress() {
        let fixture = br#"[{"iid":8,"created_at":"2024-01-02T03:04:05Z","updated_at":"2024-01-02T03:04:05Z","draft":true,"work_in_progress":false}]"#;
        assert!(parse_mrs(fixture).unwrap()[0].draft);
    }

    #[test]
    #[ignore = "requiere acceso de solo lectura a GitLab y HERDR_GITLAB_TEST_REPO"]
    fn reads_real_gitlab_repo() {
        let repo = std::env::var_os("HERDR_GITLAB_TEST_REPO")
            .map(std::path::PathBuf::from)
            .expect("falta HERDR_GITLAB_TEST_REPO");
        assert!(is_gitlab(&repo));
        for state in [
            MrState::Opened,
            MrState::Merged,
            MrState::Closed,
            MrState::All,
        ] {
            list_mrs(&repo, state, 1).unwrap();
        }
        let mrs = list_mrs(&repo, MrState::All, 1).unwrap();
        let mr = mrs.first().expect("el proyecto no tiene MRs");
        get_mr(&repo, mr.iid).unwrap();
        mr_diffs(&repo, mr.iid).unwrap();
        mr_commits(&repo, mr.iid).unwrap();
    }

    #[test]
    fn counts_diff_lines() {
        let diff = convert_diff(RawDiff {
            diff: "--- a/x\n+++ b/x\n-old\n+new\n+more\n".to_owned(),
            ..RawDiff::default()
        });
        assert_eq!((diff.additions, diff.deletions), (2, 1));
    }
}
