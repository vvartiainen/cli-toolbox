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

/// A new workspace and the pane an agent can start in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenedWorkspace {
    pub(crate) workspace_id: String,
    pub(crate) pane_id: String,
}

/// Open a workspace in `cwd` on the saved machine `machine`, labelled
/// `label`, without focusing it.
pub(crate) fn open_machine_workspace(
    machine: &str,
    cwd: &str,
    label: &str,
) -> Result<OpenedWorkspace, Error> {
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
    Ok(OpenedWorkspace {
        workspace_id: created.workspace.workspace_id,
        pane_id: created.root_pane.pane_id,
    })
}

/// The first pane of the local workspace `id`: the root pane of a workspace
/// that was just opened.
pub(crate) fn first_pane(id: &str) -> Result<String, Error> {
    let json = exec::combined_output("herdr", &["pane", "list", "--workspace", id], None)?;
    first_pane_in(&json)?.ok_or_else(|| Error::msg(format!("workspace {id} has no pane")))
}

/// Start the agent `kind`, named `name`, in `pane` on `machine` (or the
/// local server), passing `args` to the agent itself.
pub(crate) fn start_agent(
    machine: Option<&str>,
    name: &str,
    kind: &str,
    pane: &str,
    args: &[&str],
) -> Result<(), Error> {
    let mut command = vec!["agent", "start", name, "--kind", kind, "--pane", pane];
    if !args.is_empty() {
        command.push("--");
        command.extend(args);
    }
    run_on(machine, &command)
}

/// Submit `text` to the running agent `name`.
pub(crate) fn prompt_agent(machine: Option<&str>, name: &str, text: &str) -> Result<(), Error> {
    run_on(machine, &["agent", "prompt", name, text])
}

fn run_on(machine: Option<&str>, args: &[&str]) -> Result<(), Error> {
    let mut command = Vec::new();
    if let Some(machine) = machine {
        command.extend(["--machine", machine]);
    }
    command.extend(args);
    exec::combined_output("herdr", &command, None).map(drop)
}

/// Whether a herdr error is `agent_not_ready`: the agent started but waits
/// at a question, such as a folder trust prompt.
pub(crate) fn not_ready(err: &Error) -> bool {
    err.to_string().contains("agent_not_ready")
}

/// A valid herdr agent name for `label`: lowercase letters, digits, `-` and
/// `_`, starting with a letter, at most 32 characters.
pub(crate) fn agent_name(label: &str) -> String {
    let mut name: String = label
        .chars()
        .map(|ch| match ch.to_ascii_lowercase() {
            ch @ ('a'..='z' | '0'..='9' | '_' | '-') => ch,
            _ => '-',
        })
        .collect();
    if !name.starts_with(|ch: char| ch.is_ascii_lowercase()) {
        name.insert_str(0, "a-");
    }
    // Keep the end, which tells numbered worktrees apart. Only ASCII is left.
    if name.len() > 32 {
        name = format!("a-{}", &name[name.len() - 30..]);
    }
    name
}

/// The agent kinds `herdr agent start` supports, from its help.
pub(crate) fn agent_kinds() -> Vec<String> {
    exec::combined_output("herdr", &["agent", "start", "--help"], None)
        .map(|help| kinds_in(&help))
        .unwrap_or_default()
}

fn kinds_in(help: &str) -> Vec<String> {
    help.lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("[possible values: ")?
                .strip_suffix(']')
        })
        .map(|values| values.split(", ").map(String::from).collect())
        .unwrap_or_default()
}

/// Remove the saved machine labelled `label` from Herdr. Returns whether one
/// was saved.
pub(crate) fn remove_machine(label: &str) -> Result<bool, Error> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    exec::run_captured(
        "herdr",
        &["machine", "list", "--json"],
        None,
        &mut stdout,
        &mut stderr,
    )?;
    let Some(id) = machine_id(&String::from_utf8_lossy(&stdout), label)? else {
        return Ok(false);
    };
    exec::combined_output("herdr", &["machine", "remove", &id], None)?;
    Ok(true)
}

#[derive(Deserialize)]
struct SavedMachine {
    id: String,
    label: String,
}

fn machine_id(json: &str, label: &str) -> Result<Option<String>, Error> {
    let machines: Vec<SavedMachine> = serde_json::from_str(json)
        .map_err(|err| Error::msg(format!("parse herdr machine list: {err}")))?;
    Ok(machines
        .into_iter()
        .find(|machine| machine.label == label)
        .map(|machine| machine.id))
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
struct PaneListResponse {
    result: PaneList,
}

#[derive(Deserialize)]
struct PaneList {
    panes: Vec<CreatedPane>,
}

fn first_pane_in(json: &str) -> Result<Option<String>, Error> {
    serde_json::from_str::<PaneListResponse>(json)
        .map(|response| {
            response
                .result
                .panes
                .into_iter()
                .next()
                .map(|pane| pane.pane_id)
        })
        .map_err(|err| Error::msg(format!("parse herdr pane list: {err}")))
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
    fn reads_the_first_pane_of_a_workspace() {
        let json = r#"{"id":"cli:pane:list","result":{"panes":[
            {"agent":null,"cwd":"/wt","focused":true,"pane_id":"w3:p1","tab_id":"w3:t1","workspace_id":"w3"}
        ],"type":"pane_list"}}"#;
        assert_eq!(first_pane_in(json).unwrap(), Some("w3:p1".into()));
        let empty = r#"{"result":{"panes":[],"type":"pane_list"}}"#;
        assert_eq!(first_pane_in(empty).unwrap(), None);
    }

    #[test]
    fn agent_names_follow_the_herdr_rules() {
        assert_eq!(
            agent_name("cli-toolbox-agent-wt-2"),
            "cli-toolbox-agent-wt-2"
        );
        assert_eq!(agent_name("My.Repo-agent-wt"), "my-repo-agent-wt");
        assert_eq!(agent_name("2fa-agent-wt"), "a-2fa-agent-wt");
        let long = agent_name("a-very-long-repository-name-agent-wt-12");
        assert_eq!(long, "a-ng-repository-name-agent-wt-12");
        assert_eq!(long.len(), 32);
    }

    #[test]
    fn reads_the_supported_agent_kinds_from_the_help() {
        let help = "      --kind <KIND>\n          Supported agent kind\n\n          [possible values: pi, claude, codex]\n\n      --pane <ID>\n";
        assert_eq!(kinds_in(help), ["pi", "claude", "codex"]);
        assert!(kinds_in("no kinds").is_empty());
    }

    #[test]
    fn finds_a_saved_machine_by_label() {
        let json = r#"[
            {"id":"1261f5","label":"agent-demo","target":"ssh://agent@127.0.0.1:2300","session":"default","enabled":true,"selected":false},
            {"id":"9a0b1c","label":"build-box","target":"build","session":"default","enabled":true,"selected":true}
        ]"#;
        assert_eq!(
            machine_id(json, "agent-demo").unwrap(),
            Some("1261f5".into())
        );
        assert_eq!(machine_id(json, "agent-other").unwrap(), None);
        assert_eq!(machine_id("[]", "agent-demo").unwrap(), None);
        assert!(machine_id("not json", "agent-demo").is_err());
    }

    #[test]
    fn closed_or_unknown_worktrees_have_no_workspace() {
        for path in ["/repo-agent-wt-2", "/repo-agent-wt-3", "/elsewhere"] {
            assert_eq!(workspace_for(LIST, Path::new(path)).unwrap(), None);
        }
        assert!(workspace_for("not json", Path::new("/repo")).is_err());
    }
}
