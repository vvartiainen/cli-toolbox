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

/// Remove the saved SSH machine labelled `label`. Returns false when there
/// is none. Its remote session keeps running until the VM stops.
pub(crate) fn remove_machine(label: &str) -> Result<bool, Error> {
    let json = exec::combined_output("herdr", &["machine", "list", "--json"], None)?;
    let Some(id) = machine_id(&json, label)? else {
        return Ok(false);
    };
    exec::combined_output("herdr", &["machine", "remove", &id], None).map(|_| true)
}

#[derive(Deserialize)]
struct Machine {
    #[serde(alias = "profile_id")]
    id: String,
    #[serde(default)]
    label: Option<String>,
}

fn machine_id(json: &str, label: &str) -> Result<Option<String>, Error> {
    let machines: Vec<Machine> = serde_json::from_str(json)
        .map_err(|err| Error::msg(format!("parse herdr machine list: {err}")))?;
    Ok(machines
        .into_iter()
        .find(|machine| machine.label.as_deref() == Some(label))
        .map(|machine| machine.id))
}

/// Open a workspace in `cwd` on the saved machine `machine`, labelled
/// `label`. With `agent`, start that agent kind in its first pane. Returns
/// the workspace id.
pub(crate) fn open_machine_workspace(
    machine: &str,
    cwd: &str,
    label: &str,
    agent: Option<&str>,
) -> Result<String, Error> {
    let json = exec::combined_output(
        "herdr",
        &[
            "--machine",
            machine,
            "workspace",
            "create",
            "--cwd",
            cwd,
            "--label",
            label,
            "--no-focus",
        ],
        None,
    )?;
    let created = workspace_created(&json)?;
    if let Some(kind) = agent {
        // Agent names must be unique per server; one VM runs one task.
        exec::combined_output(
            "herdr",
            &[
                "--machine",
                machine,
                "agent",
                "start",
                kind,
                "--kind",
                kind,
                "--pane",
                &created.root_pane.pane_id,
            ],
            None,
        )?;
    }
    Ok(created.workspace.workspace_id)
}

#[derive(Deserialize)]
struct CreateResponse {
    result: Created,
}

#[derive(Deserialize)]
struct Created {
    workspace: CreatedWorkspace,
    root_pane: CreatedPane,
}

#[derive(Deserialize)]
struct CreatedWorkspace {
    workspace_id: String,
}

#[derive(Deserialize)]
struct CreatedPane {
    pane_id: String,
}

fn workspace_created(json: &str) -> Result<Created, Error> {
    serde_json::from_str::<CreateResponse>(json)
        .map(|response| response.result)
        .map_err(|err| Error::msg(format!("parse herdr workspace create: {err}\n{json}")))
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
    fn finds_a_machine_by_label() {
        let json =
            r#"[{"id":"m1","label":"demo-agent-wt"},{"id":"m2","label":"other"},{"id":"m3"}]"#;
        assert_eq!(machine_id(json, "other").unwrap(), Some("m2".into()));
        assert_eq!(machine_id(json, "missing").unwrap(), None);
        assert_eq!(machine_id("[]", "other").unwrap(), None);
    }

    #[test]
    fn reads_the_created_workspace_and_root_pane() {
        let json = r#"{"id":"cli:workspace:create","result":{"type":"workspace_created",
            "workspace":{"workspace_id":"w2","number":2,"label":"demo"},
            "tab":{"tab_id":"w2:t1"},
            "root_pane":{"pane_id":"w2:p1","workspace_id":"w2"}}}"#;
        let created = workspace_created(json).unwrap();
        assert_eq!(created.workspace.workspace_id, "w2");
        assert_eq!(created.root_pane.pane_id, "w2:p1");
    }

    #[test]
    fn closed_or_unknown_worktrees_have_no_workspace() {
        for path in ["/repo-agent-wt-2", "/repo-agent-wt-3", "/elsewhere"] {
            assert_eq!(workspace_for(LIST, Path::new(path)).unwrap(), None);
        }
        assert!(workspace_for("not json", Path::new("/repo")).is_err());
    }
}
