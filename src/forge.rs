//! Despacho de operaciones y textos de la forja del repositorio.
use crate::{github, gitlab, model::*};
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forge {
    GitLab,
    GitHub,
}

pub fn detect(repo: &Path) -> Option<Forge> {
    if github::is_github(repo) {
        Some(Forge::GitHub)
    } else if gitlab::is_gitlab(repo) {
        Some(Forge::GitLab)
    } else {
        None
    }
}

impl Forge {
    pub fn label(self) -> &'static str {
        match self {
            Self::GitLab => "Merge Requests",
            Self::GitHub => "Pull Requests",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Self::GitLab => "MR",
            Self::GitHub => "PR",
        }
    }
    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::GitLab => "!",
            Self::GitHub => "#",
        }
    }
    pub fn cli(self) -> &'static str {
        match self {
            Self::GitLab => "glab",
            Self::GitHub => "gh",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::GitLab => "GitLab",
            Self::GitHub => "GitHub",
        }
    }
    pub fn list_mrs(self, repo: &Path, state: MrState, page: u32) -> Result<Vec<MergeRequest>> {
        match self {
            Self::GitLab => gitlab::list_mrs(repo, state, page),
            Self::GitHub => github::list_mrs(repo, state, page),
        }
    }
    pub fn get_mr(self, repo: &Path, iid: u64) -> Result<MergeRequest> {
        match self {
            Self::GitLab => gitlab::get_mr(repo, iid),
            Self::GitHub => github::get_mr(repo, iid),
        }
    }
    pub fn mr_diffs(self, repo: &Path, iid: u64) -> Result<Vec<MrFileDiff>> {
        match self {
            Self::GitLab => gitlab::mr_diffs(repo, iid),
            Self::GitHub => github::mr_diffs(repo, iid),
        }
    }
    pub fn mr_commits(self, repo: &Path, iid: u64) -> Result<Vec<MrCommit>> {
        match self {
            Self::GitLab => gitlab::mr_commits(repo, iid),
            Self::GitHub => github::mr_commits(repo, iid),
        }
    }
}
