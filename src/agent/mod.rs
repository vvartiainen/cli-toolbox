use std::fmt;
use std::io::Write;
use std::path::Path;

use crate::git::{self, AgentWorktree};
use crate::{Error, herdr, project};

pub(crate) mod vm;

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
    pub(crate) worktree: AgentWorktree,
    pub(crate) steps: Vec<Step>,
}

/// Create the next agent worktree of `cwd` under `root`, initialize it and
/// open it in Herdr. Init output streams to `stderr`.
pub(crate) fn create_worktree(
    cwd: &Path,
    root: &Path,
    options: Options,
    stderr: &mut dyn Write,
) -> Result<Created, Error> {
    let worktree = git::create_agent_worktree_record(cwd, root, stderr)?;
    let mut steps = Vec::new();
    if options.init {
        steps.push(init_step(cwd, &worktree.path, stderr)?);
    }
    if options.herdr {
        steps.push(herdr_open_step(cwd, &worktree));
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

fn herdr_open_step(repo: &Path, worktree: &AgentWorktree) -> Step {
    if !herdr::available() {
        return Step::Skipped("herdr: not installed".into());
    }
    match herdr::open_worktree(repo, &worktree.path, &worktree.name) {
        Ok(()) => Step::Done(format!("herdr: opened workspace {}", worktree.name)),
        Err(err) => Step::Failed(format!("herdr: {err}")),
    }
}

/// Remove an agent worktree and its branch, then close its Herdr workspace.
/// The workspace stays open when git refuses the removal, for example
/// because the worktree has uncommitted changes.
pub(crate) fn remove_worktree(
    cwd: &Path,
    worktree: &AgentWorktree,
    options: Options,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    // Herdr stops listing the worktree once git has removed it.
    let lookup = options
        .herdr
        .then(|| herdr::available().then(|| herdr::worktree_workspace(cwd, &worktree.path)));
    git::remove_agent_worktree(cwd, worktree, git::RemoveOptions::default(), stderr)?;
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
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let created = create_worktree(cwd, root, options, stderr)?;
    report(&created.steps, stderr)?;
    writeln!(stdout, "{}", created.worktree.path.display())?;
    check_steps(&created.steps, "worktree created")
}

/// `agent worktree cleanup`: remove the named worktree, or all of them.
pub(crate) fn cleanup(
    cwd: &Path,
    name: &str,
    all: bool,
    options: Options,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let mut steps = Vec::new();
    for worktree in git::select_agent_worktrees(cwd, name, all)? {
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
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        let path = String::from_utf8(stdout).unwrap();
        assert!(path.trim().ends_with("demo-agent-wt"), "{path}");
        let stderr = String::from_utf8(stderr).unwrap();
        assert!(stderr.contains("– init: no `worktree-init`"), "{stderr}");

        let worktrees = git::agent_worktrees(&repo).unwrap();
        let steps = remove_worktree(&repo, &worktrees[0], NO_HERDR, &mut Vec::new()).unwrap();
        assert!(steps.is_empty());
        assert!(git::agent_worktrees(&repo).unwrap().is_empty());
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
