//! Sandboxed agent worktrees: each worktree runs its own agent-sandbox
//! microVM, added to the host herdr as a machine whose guest herdr hosts the
//! agent. The agent's commits come back as the worktree's branch.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::{Created, Step, check_steps, report};
use crate::git::{self, AgentWorktree};
use crate::sandbox::{self, RunnerState, StartOptions};
use crate::{Error, herdr};

/// Where the worktree is mounted in the guest (`agentSandbox.workspaces.app`).
pub(crate) const GUEST_DIR: &str = "/workspace/app";

/// The guest home of a new VM.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HomeFrom {
    /// Clone the main checkout's `.agent-sandbox/home.img`.
    Main,
    /// Clone this image.
    Image(PathBuf),
}

/// Choices shared by the VM commands.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct VmOptions {
    /// The sandbox flake directory. Found in the checkout when not set.
    pub(crate) flake: Option<PathBuf>,
    /// Start from a copy of another home image instead of a fresh home.
    pub(crate) home_from: Option<HomeFrom>,
    /// The agent kind herdr starts in the guest workspace, if any.
    pub(crate) agent: Option<String>,
    /// Add the VM to herdr and open a guest workspace (or remove it on done).
    pub(crate) herdr: bool,
    /// `done`: remove even with unsynced commits or local changes.
    pub(crate) force: bool,
}

const HOME_IMAGE: &str = ".agent-sandbox/home.img";

/// Create the next agent worktree of `cwd` and boot its VM.
pub(crate) fn start_worktree(
    cwd: &Path,
    root: &Path,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Created, Error> {
    let main = git::main_checkout(cwd)?;
    let flake = resolve_flake(&main, None, options.flake.as_deref())?;
    if !git::is_ignored(&main, ".agent-sandbox/") {
        return Err(Error::msg(format!(
            "add `.agent-sandbox/` to {}/.gitignore first: it holds the VM's home image, and git would refuse to remove the worktree",
            main.display()
        )));
    }
    let seed = home_seed(&main, options.home_from.as_ref())?;
    let worktree = git::create_agent_worktree_record(cwd, root, stderr)?;
    // A sandbox flake committed in the branch wins over the main checkout's.
    let flake =
        resolve_flake(&main, Some(&worktree.path), options.flake.as_deref()).unwrap_or(flake);
    let steps = boot(&worktree, &flake, seed, options, stderr)?;
    Ok(Created { worktree, steps })
}

/// `agent start`: print the worktree path on stdout and the steps on stderr.
pub(crate) fn start(
    cwd: &Path,
    root: &Path,
    options: &VmOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let created = start_worktree(cwd, root, options, stderr)?;
    report(&created.steps, stderr)?;
    writeln!(stdout, "{}", created.worktree.path.display())?;
    check_steps(&created.steps, "agent worktree created")
}

/// `agent up`: boot the VM of an existing worktree again.
pub(crate) fn up(
    cwd: &Path,
    name: &str,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    let worktree = select(cwd, name)?;
    let flake = worktree_flake(cwd, &worktree, options)?;
    if let Some(vm) = sandbox::vm_for(&runtime_root()?, &worktree.path) {
        return Ok(vec![Step::Skipped(format!(
            "vm: already running{}",
            port_text(&vm)
        ))]);
    }
    let main = git::main_checkout(cwd)?;
    let seed = home_seed(&main, options.home_from.as_ref())?;
    boot(&worktree, &flake, seed, options, stderr)
}

/// `agent stop`: stop the VM of a worktree and keep everything else.
pub(crate) fn stop(cwd: &Path, name: &str, options: &VmOptions) -> Result<Vec<Step>, Error> {
    let worktree = select(cwd, name)?;
    let flake = worktree_flake(cwd, &worktree, options)?;
    if sandbox::vm_for(&runtime_root()?, &worktree.path).is_none() {
        return Ok(vec![Step::Skipped("vm: not running".into())]);
    }
    sandbox::stop(&worktree.path, &flake)?;
    Ok(vec![Step::Done(format!("vm: stopped {}", worktree.name))])
}

/// The result of bringing the agent's commits to the host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Synced {
    pub(crate) worktree: AgentWorktree,
    /// What git-pull reported, one line per workspace.
    pub(crate) report: String,
    /// Guest commits that the host branch does not have, one per line.
    pub(crate) unsynced: Option<String>,
}

/// Fetch the guest's commits and fast-forward the worktree's branch.
pub(crate) fn sync_worktree(cwd: &Path, name: &str, options: &VmOptions) -> Result<Synced, Error> {
    let worktree = select(cwd, name)?;
    let flake = worktree_flake(cwd, &worktree, options)?;
    if sandbox::vm_for(&runtime_root()?, &worktree.path).is_none() {
        return Err(Error::msg(format!(
            "the VM of {} is not running; start it with `cli-toolbox agent up {}`",
            worktree.name, worktree.name
        )));
    }
    let report = sandbox::git_pull(&worktree.path, &flake)?;
    let unsynced = unsynced_commits(&worktree.path, &worktree.branch)?;
    Ok(Synced {
        worktree,
        report,
        unsynced,
    })
}

/// `agent sync`: report what moved and the branch's commits over main.
pub(crate) fn sync(
    cwd: &Path,
    name: &str,
    options: &VmOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let synced = sync_worktree(cwd, name, options)?;
    write!(stderr, "{}", synced.report)?;
    let main = git::main_checkout(cwd)?;
    let branch = &synced.worktree.branch;
    let log = git::log_oneline(&main, &format!("HEAD..{branch}"))?;
    if log.is_empty() {
        writeln!(
            stdout,
            "{branch}: no commits beyond the main checkout's HEAD"
        )?;
    } else {
        writeln!(
            stdout,
            "{branch}: commits beyond the main checkout's HEAD:\n{log}"
        )?;
    }
    if let Some(unsynced) = &synced.unsynced {
        writeln!(
            stderr,
            "{branch} has diverged from the guest; its commits are in sandbox/{branch}:\n{unsynced}"
        )?;
    }
    Ok(())
}

/// Guest commits fetched into `refs/remotes/sandbox/<branch>` that `branch`
/// does not contain, for example after the host committed on its own.
pub(crate) fn unsynced_commits(dir: &Path, branch: &str) -> Result<Option<String>, Error> {
    let guest = format!("refs/remotes/sandbox/{branch}");
    if git::resolve_commit(dir, &guest).is_none() || git::is_ancestor(dir, &guest, branch) {
        return Ok(None);
    }
    git::log_oneline(dir, &format!("{branch}..{guest}")).map(Some)
}

/// `agent done`: sync, then stop the VM, remove the herdr machine and the
/// worktree. The branch stays for review. Refuses to lose work unless forced.
pub(crate) fn done(
    cwd: &Path,
    name: &str,
    options: &VmOptions,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let worktree = select(cwd, name)?;
    let flake = worktree_flake(cwd, &worktree, options)?;
    let vm = sandbox::vm_for(&runtime_root()?, &worktree.path);
    if let Some(problem) = done_blocker(cwd, &worktree, vm.as_ref(), options)? {
        return Err(Error::msg(format!(
            "{problem}\nPass --force to remove {} anyway; its branch {} stays.",
            worktree.name, worktree.branch
        )));
    }

    let mut steps = Vec::new();
    if let Some(vm) = &vm {
        steps.push(match sandbox::stop(&worktree.path, &flake) {
            Ok(()) => {
                // Only the runtime dir's ssh_config and known_hosts remain.
                let _ = std::fs::remove_dir_all(&vm.runtime_dir);
                Step::Done(format!("vm: stopped {}", worktree.name))
            }
            Err(err) => Step::Failed(format!("vm: {err}")),
        });
        if steps.iter().any(Step::failed) {
            report(&steps, stderr)?;
            return check_steps(&steps, "nothing removed");
        }
    }
    if options.herdr {
        steps.push(remove_machine_step(&worktree.name));
    }
    let remove = git::RemoveOptions {
        keep_branch: true,
        // Uncommitted and untracked files were checked above.
        force: true,
    };
    git::remove_agent_worktree(cwd, &worktree, remove, stderr)?;
    steps.push(Step::Done(format!(
        "worktree: removed; branch {} kept for review",
        worktree.branch
    )));
    report(&steps, stderr)?;
    writeln!(stdout, "{}", worktree.branch)?;
    check_steps(&steps, "agent worktree removed")
}

/// Why `done` would lose work, if it would.
fn done_blocker(
    cwd: &Path,
    worktree: &AgentWorktree,
    vm: Option<&RunnerState>,
    options: &VmOptions,
) -> Result<Option<String>, Error> {
    if options.force {
        return Ok(None);
    }
    if vm.is_some() {
        let synced = sync_worktree(cwd, &worktree.name, options)?;
        if let Some(unsynced) = synced.unsynced {
            return Ok(Some(format!(
                "{} has diverged from the guest; these guest commits would be lost:\n{unsynced}",
                worktree.branch
            )));
        }
    } else if worktree.path.join(HOME_IMAGE).exists() {
        return Ok(Some(format!(
            "the VM of {} is stopped, so its latest commits cannot be synced; run `cli-toolbox agent up {}` first",
            worktree.name, worktree.name
        )));
    }
    let changes = git::status_lines(&worktree.path)?;
    if !changes.is_empty() {
        return Ok(Some(format!(
            "{} has uncommitted changes; commit them in the guest and run done again:\n{}",
            worktree.name,
            changes.join("\n")
        )));
    }
    Ok(None)
}

/// `agent list`: one line per agent worktree with its VM.
pub(crate) fn list(cwd: &Path, stdout: &mut dyn Write) -> Result<(), Error> {
    let running = runtime_root()
        .map(|root| sandbox::running_vms(&root))
        .unwrap_or_default();
    for worktree in git::agent_worktrees(cwd)? {
        writeln!(stdout, "{}", list_line(&worktree, &running))?;
    }
    Ok(())
}

fn list_line(worktree: &AgentWorktree, running: &[RunnerState]) -> String {
    let path = worktree
        .path
        .canonicalize()
        .unwrap_or_else(|_| worktree.path.clone());
    let vm = running
        .iter()
        .find(|vm| vm.dir == path)
        .map(|vm| format!("running{}", port_text(vm)))
        .unwrap_or_else(|| "stopped".into());
    format!(
        "{}\t{}\t{vm}\t{}",
        worktree.name,
        worktree.branch,
        worktree.path.display()
    )
}

fn boot(
    worktree: &AgentWorktree,
    flake: &Path,
    home_seed: Option<PathBuf>,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    let running = sandbox::running_vms(&runtime_root()?);
    let port = sandbox::free_port(sandbox::SSH_PORTS, &running).ok_or_else(|| {
        Error::msg(format!(
            "no free SSH port in {}-{}",
            sandbox::SSH_PORTS.start(),
            sandbox::SSH_PORTS.end()
        ))
    })?;
    writeln!(
        stderr,
        "Starting the VM of {} (SSH port {port}) from {}",
        worktree.name,
        flake.display()
    )?;
    let start = StartOptions {
        ssh_port: Some(port),
        name: Some(worktree.name.clone()),
        home_seed,
    };
    let mut steps = Vec::new();
    match sandbox::start(&worktree.path, flake, &start) {
        Ok(vm) => steps.push(Step::Done(format!("vm: running{}", port_text(&vm)))),
        Err(err) => {
            steps.push(Step::Failed(format!("vm: {err}")));
            return Ok(steps);
        }
    }
    if options.herdr {
        steps.extend(herdr_steps(worktree, flake, options.agent.as_deref()));
    }
    Ok(steps)
}

fn herdr_steps(worktree: &AgentWorktree, flake: &Path, agent: Option<&str>) -> Vec<Step> {
    if !herdr::available() {
        return vec![Step::Skipped("herdr: not installed".into())];
    }
    let label = &worktree.name;
    if let Err(err) = sandbox::herdr_add(&worktree.path, flake, label) {
        return vec![Step::Failed(format!("herdr: add machine {label}: {err}"))];
    }
    let added = Step::Done(format!("herdr: machine {label}"));
    let opened = match herdr::open_machine_workspace(label, GUEST_DIR, label, agent) {
        Ok(id) => Step::Done(match agent {
            Some(kind) => format!("herdr: workspace {id} running {kind}"),
            None => {
                format!("herdr: workspace {id} (no agent; pass --agent or set CLI_TOOLBOX_AGENT)")
            }
        }),
        Err(err) => Step::Failed(format!("herdr: open workspace on {label}: {err}")),
    };
    vec![added, opened]
}

fn remove_machine_step(label: &str) -> Step {
    if !herdr::available() {
        return Step::Skipped("herdr: not installed".into());
    }
    match herdr::remove_machine(label) {
        Ok(true) => Step::Done(format!("herdr: removed machine {label}")),
        Ok(false) => Step::Skipped(format!("herdr: no machine {label}")),
        Err(err) => Step::Failed(format!("herdr: remove machine {label}: {err}")),
    }
}

fn port_text(vm: &RunnerState) -> String {
    vm.ssh_port
        .map(|port| format!(" on SSH port {port}"))
        .unwrap_or_default()
}

fn select(cwd: &Path, name: &str) -> Result<AgentWorktree, Error> {
    let mut selected = git::select_agent_worktrees(cwd, name, false)?;
    Ok(selected.remove(0))
}

fn worktree_flake(
    cwd: &Path,
    worktree: &AgentWorktree,
    options: &VmOptions,
) -> Result<PathBuf, Error> {
    let main = git::main_checkout(cwd)?;
    resolve_flake(&main, Some(&worktree.path), options.flake.as_deref())
}

/// The sandbox flake: the one given, the worktree's, then the main checkout's.
fn resolve_flake(
    main: &Path,
    worktree: Option<&Path>,
    given: Option<&Path>,
) -> Result<PathBuf, Error> {
    if let Some(flake) = given {
        return std::path::absolute(flake)
            .map_err(|err| Error::msg(format!("resolve {}: {err}", flake.display())));
    }
    worktree
        .and_then(sandbox::find_flake)
        .or_else(|| sandbox::find_flake(main))
        .ok_or_else(|| {
            Error::msg(format!(
                "no agent-sandbox flake in {} (sandbox/flake.nix); create one with `cli-toolbox agent sandbox init`",
                main.display()
            ))
        })
}

fn home_seed(main: &Path, from: Option<&HomeFrom>) -> Result<Option<PathBuf>, Error> {
    let image = match from {
        None => return Ok(None),
        Some(HomeFrom::Main) => main.join(HOME_IMAGE),
        Some(HomeFrom::Image(path)) => std::path::absolute(path)
            .map_err(|err| Error::msg(format!("resolve {}: {err}", path.display())))?,
    };
    if image.is_file() {
        Ok(Some(image))
    } else {
        Err(Error::msg(format!(
            "no home image to copy at {}; run the sandbox there once first",
            image.display()
        )))
    }
}

fn runtime_root() -> Result<PathBuf, Error> {
    sandbox::runtime_root().ok_or_else(|| {
        Error::msg("resolve agent-sandbox runtime directory: home directory not found")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{commit_all, init_git_repo, skip_without_git};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    #[test]
    fn synced_guest_commits_are_not_reported() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let branch = git(&repo, &["symbolic-ref", "--short", "HEAD"]);
        // No sync yet, then a sync that fast-forwarded the branch.
        assert_eq!(unsynced_commits(&repo, &branch).unwrap(), None);
        git(
            &repo,
            &[
                "update-ref",
                &format!("refs/remotes/sandbox/{branch}"),
                "HEAD",
            ],
        );
        assert_eq!(unsynced_commits(&repo, &branch).unwrap(), None);
    }

    #[test]
    fn diverged_guest_commits_are_reported() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let branch = git(&repo, &["symbolic-ref", "--short", "HEAD"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        // The guest committed on the base, and the host on its own.
        std::fs::write(repo.join("guest.txt"), "guest\n").unwrap();
        commit_all(&repo);
        git(
            &repo,
            &[
                "update-ref",
                &format!("refs/remotes/sandbox/{branch}"),
                "HEAD",
            ],
        );
        git(&repo, &["reset", "-q", "--hard", &base]);
        std::fs::write(repo.join("host.txt"), "host\n").unwrap();
        commit_all(&repo);

        let unsynced = unsynced_commits(&repo, &branch).unwrap().unwrap();
        assert_eq!(unsynced.lines().count(), 1, "{unsynced}");
    }

    #[test]
    fn home_seed_needs_an_existing_image() {
        let main = tempfile::tempdir().unwrap();
        assert_eq!(home_seed(main.path(), None).unwrap(), None);
        assert!(home_seed(main.path(), Some(&HomeFrom::Main)).is_err());
        std::fs::create_dir(main.path().join(".agent-sandbox")).unwrap();
        std::fs::write(main.path().join(HOME_IMAGE), "home").unwrap();
        assert_eq!(
            home_seed(main.path(), Some(&HomeFrom::Main)).unwrap(),
            Some(main.path().join(HOME_IMAGE))
        );
    }

    #[test]
    fn flake_in_the_worktree_wins_over_main() {
        let main = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        assert!(resolve_flake(main.path(), Some(worktree.path()), None).is_err());
        for dir in [main.path(), worktree.path()] {
            std::fs::create_dir(dir.join("sandbox")).unwrap();
            std::fs::write(dir.join("sandbox/flake.nix"), "agent-sandbox").unwrap();
        }
        assert_eq!(
            resolve_flake(main.path(), Some(worktree.path()), None).unwrap(),
            worktree.path().join("sandbox")
        );
        assert_eq!(
            resolve_flake(main.path(), None, None).unwrap(),
            main.path().join("sandbox")
        );
    }

    #[test]
    fn done_refuses_uncommitted_changes_and_stopped_vms_with_a_home() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        std::fs::write(repo.join(".gitignore"), ".agent-sandbox/\n").unwrap();
        commit_all(&repo);
        let worktree = git::create_agent_worktree_record(
            &repo,
            &temp.path().join("worktrees"),
            &mut Vec::new(),
        )
        .unwrap();
        let options = VmOptions::default();
        assert_eq!(
            done_blocker(&repo, &worktree, None, &options).unwrap(),
            None
        );

        std::fs::write(worktree.path.join("wip.txt"), "wip\n").unwrap();
        let blocker = done_blocker(&repo, &worktree, None, &options)
            .unwrap()
            .unwrap();
        assert!(blocker.contains("?? wip.txt"), "{blocker}");

        std::fs::create_dir(worktree.path.join(".agent-sandbox")).unwrap();
        std::fs::write(worktree.path.join(HOME_IMAGE), "home").unwrap();
        let blocker = done_blocker(&repo, &worktree, None, &options)
            .unwrap()
            .unwrap();
        assert!(blocker.contains("is stopped"), "{blocker}");

        let forced = VmOptions {
            force: true,
            ..VmOptions::default()
        };
        assert_eq!(done_blocker(&repo, &worktree, None, &forced).unwrap(), None);
    }

    #[test]
    fn list_shows_the_vm_of_each_worktree() {
        let worktree = AgentWorktree {
            name: "demo-agent-wt".into(),
            path: "/wt/demo-agent-wt".into(),
            branch: "demo-agent-wt".into(),
        };
        let vm = RunnerState {
            pid: 1,
            ssh_port: Some(2301),
            name: "demo-agent-wt".into(),
            dir: "/wt/demo-agent-wt".into(),
            runtime_dir: "/state".into(),
            console_log: None,
        };
        assert_eq!(
            list_line(&worktree, &[vm]),
            "demo-agent-wt\tdemo-agent-wt\trunning on SSH port 2301\t/wt/demo-agent-wt"
        );
        assert_eq!(
            list_line(&worktree, &[]),
            "demo-agent-wt\tdemo-agent-wt\tstopped\t/wt/demo-agent-wt"
        );
    }
}
