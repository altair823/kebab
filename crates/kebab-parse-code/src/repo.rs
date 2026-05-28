//! Git repo auto-detection (spec §5.1).
//!
//! Walks up from `path` looking for a `.git/` directory. If found, reads
//! repo dir name, current branch, and HEAD commit using `gix` (pure Rust;
//! no `git` binary on PATH required).

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoMeta {
    pub name: String,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

/// Walk up from `path` until a `.git/` directory is found. Returns repo
/// metadata, or `None` if no repo boundary is reached before the filesystem
/// root.
///
/// - `name`: directory name containing `.git/`.
/// - `branch`: current HEAD branch, or `"detached"` if detached HEAD, or
///   `None` if branch can't be read.
/// - `commit`: 40-hex commit SHA at HEAD, or `None` if empty repo / read
///   failure.
///
/// `.git/` as a file (worktree marker / submodule) returns `None` for
/// `branch` and `commit` and falls back to the parent dir name for `name`.
pub fn detect_repo(path: &Path) -> Option<RepoMeta> {
    let mut cur = if path.is_dir() { path } else { path.parent()? };
    loop {
        let dotgit = cur.join(".git");
        if dotgit.is_dir() {
            let name = cur.file_name()?.to_string_lossy().into_owned();
            let (branch, commit) = read_head(cur);
            return Some(RepoMeta {
                name,
                branch,
                commit,
            });
        } else if dotgit.is_file() {
            let name = cur.file_name()?.to_string_lossy().into_owned();
            return Some(RepoMeta {
                name,
                branch: None,
                commit: None,
            });
        }
        cur = cur.parent()?;
    }
}

fn read_head(repo_dir: &Path) -> (Option<String>, Option<String>) {
    match gix::open(repo_dir) {
        Ok(repo) => {
            let branch = repo
                .head_name()
                .ok()
                .flatten()
                .map(|n| n.shorten().to_string())
                .or_else(|| Some("detached".to_string()));
            let commit = repo.head_id().ok().map(|id| id.to_string());
            (branch, commit)
        }
        Err(_) => (None, None),
    }
}
