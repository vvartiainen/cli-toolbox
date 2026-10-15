use std::path::Path;

use serde::Deserialize;

use crate::{Error, exec};

/// Whether the `herdr` CLI is installed. Herdr steps are skipped otherwise.
pub(crate) fn available() -> bool {
    exec::look_path("herdr").is_some()
}

/// Open an existing git worktree of `repo` as a Herdr workspace and focus it.
pub(crate) fn open_worktree(repo: &Path, path: &Path, label: &str) -> Result<(), Error> {
    exec::combined_output(
        "herdr",
        &[
            "worktree",
            "open",
            "--cwd",
            utf8(repo)?,
            "--path",
            utf8(path)?,
            "--label",
            label,
            "--focus",
        ],
        None,
    )
    .map(drop)
}

/// The Herdr workspace that has the worktree at `path` open, if any.
pub(crate) fn worktree_workspace(repo: &Path, path: &Path) -> Result<Option<String>, Error> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    exec::run_captured(
        "herdr",
        &["worktree", "list", "--cwd", utf8(repo)?],
        None,
        &mut stdout,
        &mut stderr,
    )?;
    workspace_for(&String::from_utf8_lossy(&stdout), path)
}

pub(crate) fn close_workspace(id: &str) -> Result<(), Error> {
    exec::combined_output("herdr", &["workspace", "close", id], None).map(drop)
}

#[derive(Deserialize)]
struct ListResponse {
    result: ListResult,
}

#[derive(Deserialize)]
struct ListResult {
    worktrees: Vec<ListedWorktree>,
}

#[derive(Deserialize)]
struct ListedWorktree {
    path: std::path::PathBuf,
    #[serde(default)]
    open_workspace_id: Option<String>,
}

fn workspace_for(json: &str, path: &Path) -> Result<Option<String>, Error> {
    let response: ListResponse = serde_json::from_str(json)
        .map_err(|err| Error::msg(format!("parse herdr worktree list: {err}")))?;
    let wanted = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    Ok(response
        .result
        .worktrees
        .into_iter()
        .find(|worktree| {
            worktree.path == path
                || worktree
                    .path
                    .canonicalize()
                    .is_ok_and(|listed| listed == wanted)
        })
        .and_then(|worktree| worktree.open_workspace_id))
}

fn utf8(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or_else(|| Error::msg(format!("herdr needs a UTF-8 path: {}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{"id":"cli:worktree:list","result":{"source":{"repo_root":"/repo"},"type":"worktree_list","worktrees":[
        {"branch":"main","path":"/repo","open_workspace_id":"wJ"},
        {"branch":"repo-agent-wt","path":"/repo-agent-wt","open_workspace_id":"wK"},
        {"branch":"repo-agent-wt-2","path":"/repo-agent-wt-2","open_workspace_id":null},
        {"branch":"repo-agent-wt-3","path":"/repo-agent-wt-3"}
    ]}}"#;

    #[test]
    fn finds_the_open_workspace_of_a_worktree() {
        assert_eq!(
            workspace_for(LIST, Path::new("/repo-agent-wt")).unwrap(),
            Some("wK".into())
        );
    }

    #[test]
    fn closed_or_unknown_worktrees_have_no_workspace() {
        for path in ["/repo-agent-wt-2", "/repo-agent-wt-3", "/elsewhere"] {
            assert_eq!(workspace_for(LIST, Path::new(path)).unwrap(), None);
        }
        assert!(workspace_for("not json", Path::new("/repo")).is_err());
    }
}
