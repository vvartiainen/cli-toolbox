use std::fmt;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git::{self, Worktree};
use crate::{Error, herdr, project};

pub(crate) mod review;
pub(crate) mod vm;

/// Kinds that take the interactive session's first prompt as an argument.
/// They run it once past a startup question such as folder trust, which
/// `herdr agent prompt` cannot get past.
const PROMPT_ARG_KINDS: [&str; 2] = ["claude", "codex"];

/// Longer prompts go through `herdr agent prompt`: macOS rejects command
/// lines over its terminal input limit of 1024 bytes.
const PROMPT_ARG_MAX: usize = 768;

/// The coding agent to start in a new worktree's Herdr workspace, and the
/// first prompt to give it.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct Launch {
    /// A `herdr agent start` kind, such as claude or codex.
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) prompt: Option<String>,
}

impl Launch {
    /// `kind`, with `prompt` unless it is blank.
    pub(crate) fn new(kind: impl Into<String>, prompt: Option<String>) -> Self {
        Self {
            kind: kind.into(),
            prompt: prompt.filter(|prompt| !prompt.trim().is_empty()),
        }
    }

    /// The prompt as the agent's own argument, when its kind takes it so.
    fn prompt_arg(&self) -> Option<&str> {
        self.prompt.as_deref().filter(|prompt| {
            PROMPT_ARG_KINDS.contains(&self.kind.as_str())
                && prompt.len() <= PROMPT_ARG_MAX
                && !prompt.contains('\n')
        })
    }

    fn text(&self) -> String {
        match self.prompt {
            Some(_) => format!("{} with the prompt", self.kind),
            None => self.kind.clone(),
        }
    }
}

/// Start `launch` in `pane` on `machine` (or the local Herdr), named after
/// the worktree `label`, and give it its prompt. Returns what runs, for
/// steps.
pub(crate) fn start_agent(
    machine: Option<&str>,
    label: &str,
    pane: &str,
    launch: &Launch,
) -> Result<String, Error> {
    // Agent names must be unique per server; one runs per worktree.
    let name = herdr::agent_name(label);
    let arg = launch.prompt_arg();
    let started = herdr::start_agent(machine, &name, &launch.kind, pane, arg.as_slice());
    match (started, arg, launch.prompt.as_deref()) {
        (Err(err), None, Some(_)) if herdr::not_ready(&err) => Err(Error::msg(format!(
            "{} waits for input, so the prompt was not sent; answer it, then paste the prompt",
            launch.kind
        ))),
        (Err(err), ..) if herdr::not_ready(&err) => {
            Ok(format!("{} (it waits for input first)", launch.text()))
        }
        (Err(err), ..) => Err(err),
        (Ok(()), None, Some(prompt)) => {
            herdr::prompt_agent(machine, &name, prompt)?;
            Ok(launch.text())
        }
        (Ok(()), ..) => Ok(launch.text()),
    }
}

/// Optional steps around creating and removing agent worktrees.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Options {
    /// Initialize the new worktree (`just worktree-init` or a package install).
    pub(crate) init: bool,
    /// Open the new worktree in Herdr, and close its workspace on removal.
    pub(crate) herdr: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            init: true,
            herdr: true,
        }
    }
}

/// The result of one optional step. A failed step does not undo the worktree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Step {
    Done(String),
    Skipped(String),
    Failed(String),
}

impl Step {
    pub(crate) fn failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// The step with `name: ` before its text, for steps of several projects.
    pub(crate) fn prefixed(self, name: &str) -> Self {
        match self {
            Self::Done(text) => Self::Done(format!("{name}: {text}")),
            Self::Skipped(text) => Self::Skipped(format!("{name}: {text}")),
            Self::Failed(text) => Self::Failed(format!("{name}: {text}")),
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Done(text) => write!(f, "✓ {text}"),
            Self::Skipped(text) => write!(f, "– {text}"),
            Self::Failed(text) => write!(f, "✗ {text}"),
        }
    }
}

pub(crate) struct Created {
    pub(crate) worktree: Worktree,
    pub(crate) steps: Vec<Step>,
}

/// Create the next agent worktree of `cwd` under `root`, initialize it and
/// open it in Herdr, starting `launch` there. Init output streams to
/// `stderr`.
pub(crate) fn create_worktree(
    cwd: &Path,
    root: &Path,
    options: Options,
    launch: Option<&Launch>,
    stderr: &mut dyn Write,
) -> Result<Created, Error> {
    let name = git::next_agent_worktree_name(cwd, root)?;
    let worktree = git::create_worktree(cwd, root, &name, stderr)?;
    let mut steps = Vec::new();
    if options.init {
        steps.push(init_step(cwd, &worktree.path, stderr)?);
    }
    if options.herdr {
        steps.push(herdr_open_step(cwd, &worktree, launch));
    } else if let Some(launch) = launch {
        steps.push(Step::Skipped(format!(
            "agent: {} needs Herdr to run in",
            launch.kind
        )));
    }
    Ok(Created { worktree, steps })
}

fn init_step(source: &Path, dir: &Path, stderr: &mut dyn Write) -> Result<Step, Error> {
    let Some(command) = project::detect_init(dir) else {
        return Ok(Step::Skipped(format!(
            "init: no `{}` just recipe or package lockfile",
            project::INIT_RECIPE
        )));
    };
    writeln!(stderr, "Initializing {}: {command}", dir.display())?;
    Ok(match project::run_init(&command, dir, source) {
        Ok(()) => Step::Done(format!("init: {command}")),
        Err(err) => Step::Failed(format!("init: {err}")),
    })
}

fn herdr_open_step(repo: &Path, worktree: &Worktree, launch: Option<&Launch>) -> Step {
    if !herdr::available() {
        return Step::Skipped("herdr: not installed".into());
    }
    if let Err(err) = herdr::open_worktree(repo, &worktree.path, &worktree.name) {
        return Step::Failed(format!("herdr: {err}"));
    }
    let Some(launch) = launch else {
        return Step::Done(format!("herdr: opened workspace {}", worktree.name));
    };
    match start_in_worktree(repo, worktree, launch) {
        Ok(running) => Step::Done(format!(
            "herdr: opened workspace {} running {running}",
            worktree.name
        )),
        Err(err) => Step::Failed(format!(
            "herdr: opened workspace {}, but could not start {}: {err}",
            worktree.name, launch.kind
        )),
    }
}

/// Start `launch` in the root pane of the worktree's new workspace.
fn start_in_worktree(repo: &Path, worktree: &Worktree, launch: &Launch) -> Result<String, Error> {
    let id = herdr::worktree_workspace(repo, &worktree.path)?
        .ok_or_else(|| Error::msg("herdr lists no open workspace for it"))?;
    let pane = herdr::first_pane(&id)?;
    start_agent(None, &worktree.name, &pane, launch)
}

/// Remove an agent worktree and its branch, then close its Herdr workspace.
/// The workspace stays open when git refuses the removal, for example
/// because the worktree has uncommitted changes.
pub(crate) fn remove_worktree(
    cwd: &Path,
    worktree: &Worktree,
    options: Options,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    // Herdr stops listing the worktree once git has removed it.
    let lookup = options
        .herdr
        .then(|| herdr::available().then(|| herdr::worktree_workspace(cwd, &worktree.path)));
    git::remove_worktree(cwd, worktree, git::RemoveOptions::default(), stderr)?;
    let Some(lookup) = lookup else {
        return Ok(Vec::new());
    };
    let step = match lookup {
        None => Step::Skipped("herdr: not installed".into()),
        Some(Err(err)) => Step::Failed(format!("herdr: find workspace: {err}")),
        Some(Ok(None)) => Step::Skipped("herdr: no open workspace".into()),
        Some(Ok(Some(id))) => match herdr::close_workspace(&id) {
            Ok(()) => Step::Done(format!("herdr: closed workspace {id}")),
            Err(err) => Step::Failed(format!("herdr: close workspace {id}: {err}")),
        },
    };
    Ok(vec![step])
}

/// `agent worktree create`: print the path on stdout for `cd "$(…)"`, and
/// the steps on stderr. The path is printed even when a step fails.
pub(crate) fn create(
    cwd: &Path,
    root: &Path,
    options: Options,
    launch: Option<&Launch>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let created = create_worktree(cwd, root, options, launch, stderr)?;
    report(&created.steps, stderr)?;
    writeln!(stdout, "{}", created.worktree.path.display())?;
    check_steps(&created.steps, "worktree created")
}

/// `agent worktree cleanup`: remove the named worktree, or all of them.
pub(crate) fn cleanup(
    cwd: &Path,
    root: &Path,
    name: &str,
    all: bool,
    options: Options,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let mut steps = Vec::new();
    for worktree in git::select_worktrees(git::project_worktrees(cwd, root)?, name, all)? {
        let removed = remove_worktree(cwd, &worktree, options, stderr)?;
        report(&removed, stderr)?;
        writeln!(stdout, "{}", worktree.path.display())?;
        steps.extend(removed);
    }
    check_steps(&steps, "worktree removed")
}

fn report(steps: &[Step], stderr: &mut dyn Write) -> Result<(), Error> {
    for step in steps {
        writeln!(stderr, "{step}")?;
    }
    Ok(())
}

fn check_steps(steps: &[Step], done: &str) -> Result<(), Error> {
    if steps.iter().any(Step::failed) {
        Err(Error::msg(format!("{done}, but a step failed")))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{init_git_repo, skip_without_git};

    const NO_HERDR: Options = Options {
        init: true,
        herdr: false,
    };

    #[test]
    fn create_runs_the_worktree_init_recipe_in_the_new_worktree() {
        if skip_without_git() || crate::exec::look_path("just").is_none() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        std::fs::write(
            repo.join("justfile"),
            "worktree-init:\n    printf %s \"$CLI_TOOLBOX_WORKTREE_SOURCE\" > initialized-from\n",
        )
        .unwrap();
        crate::git::tests::commit_all(&repo);

        let created = create_worktree(
            &repo,
            &root.path().join("worktrees"),
            NO_HERDR,
            None,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            created.steps,
            vec![Step::Done("init: just worktree-init".into())]
        );
        let source =
            std::fs::read_to_string(created.worktree.path.join("initialized-from")).unwrap();
        assert_eq!(Path::new(&source), repo);
    }

    #[test]
    fn create_skips_init_for_plain_projects_and_cleanup_removes() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        create(
            &repo,
            &root.path().join("worktrees"),
            NO_HERDR,
            None,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let path = String::from_utf8(stdout).unwrap();
        assert!(path.trim().ends_with("demo-agent-wt"), "{path}");
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("– init: no `worktree-init`"), "{stderr}");

        let worktrees = git::project_worktrees(&repo, &root.path().join("worktrees")).unwrap();
        let steps = remove_worktree(&repo, &worktrees[0], NO_HERDR, &mut Vec::new()).unwrap();
        assert!(steps.is_empty());
        assert!(
            git::project_worktrees(&repo, &root.path().join("worktrees"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn short_one_line_prompts_go_to_claude_and_codex_as_an_argument() {
        let launch = |kind: &str, prompt: &str| Launch::new(kind, Some(prompt.into()));
        assert_eq!(launch("claude", "Fix #12").prompt_arg(), Some("Fix #12"));
        assert_eq!(launch("codex", "Fix #12").prompt_arg(), Some("Fix #12"));
        assert_eq!(launch("pi", "Fix #12").prompt_arg(), None);
        assert_eq!(launch("claude", "Fix\nall").prompt_arg(), None);
        assert_eq!(launch("claude", &"x".repeat(800)).prompt_arg(), None);
        assert_eq!(Launch::new("claude", Some("  ".into())).prompt, None);
        assert_eq!(launch("codex", "go").text(), "codex with the prompt");
    }

    #[test]
    fn an_agent_without_herdr_is_reported_as_skipped() {
        if skip_without_git() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("demo");
        init_git_repo(&repo);
        let options = Options {
            init: false,
            herdr: false,
        };
        let launch = Launch::new("codex", Some("Add tests".into()));
        let created = create_worktree(
            &repo,
            &root.path().join("worktrees"),
            options,
            Some(&launch),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            created.steps,
            vec![Step::Skipped("agent: codex needs Herdr to run in".into())]
        );
    }

    #[test]
    fn failed_steps_fail_the_command_after_reporting() {
        let steps = [
            Step::Done("init: npm ci".into()),
            Step::Failed("herdr: no server".into()),
        ];
        let mut stderr = Vec::new();
        report(&steps, &mut stderr).unwrap();
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "✓ init: npm ci\n✗ herdr: no server\n"
        );
        assert!(check_steps(&steps, "worktree created").is_err());
        assert!(check_steps(&steps[..1], "worktree created").is_ok());
    }
}
