//! Sandbox projects: one agent-sandbox microVM per project, defined in the
//! central sandbox directory and mounting every agent worktree of the
//! project's repository at `/workspace/<worktree>`. The VM is added to the
//! host herdr as a machine, with a guest workspace per worktree. The agents'
//! commits come back as the worktrees' branches.
//!
//! The `vm_*` functions manage every running agent-sandbox VM from its
//! runner state alone, also when its project directory is gone.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use super::{Created, Launch, Step};
use crate::git::{self, AgentWorktree};
use crate::sandbox::project::{self, CreateOptions, Project};
use crate::sandbox::setup::{self, Backlog};
use crate::sandbox::{self, RunnerState};
use crate::{Error, herdr, select};

/// Each worktree's own agent and its first prompt until it is sent, by
/// worktree name, next to the flake.
const AGENTS_FILE: &str = "agents.json";

/// Choices shared by the project commands.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct VmOptions {
    /// The agent kind herdr starts in each guest workspace that has no agent
    /// of its own, if any.
    pub(crate) agent: Option<String>,
    /// Add the VM to herdr and open a guest workspace per worktree.
    pub(crate) herdr: bool,
    /// `done`: remove even with unsynced commits or local changes.
    pub(crate) force: bool,
}

/// A project with its worktrees and VM, for lists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectStatus {
    pub(crate) project: Project,
    pub(crate) worktrees: Vec<AgentWorktree>,
    pub(crate) vm: Option<RunnerState>,
    /// The running VM does not mount the current worktrees.
    pub(crate) restart_needed: bool,
    /// Why the worktrees could not be read, for example a moved repository.
    pub(crate) error: Option<String>,
}

impl ProjectStatus {
    pub(crate) fn vm_text(&self) -> String {
        match &self.vm {
            Some(vm) if self.restart_needed => {
                format!(
                    "running{}, restart to mount worktree changes",
                    port_text(vm)
                )
            }
            Some(vm) => format!("running{}", port_text(vm)),
            None => "stopped".into(),
        }
    }
}

/// Every project in `root` with its state.
pub(crate) fn statuses(root: &Path) -> Result<Vec<ProjectStatus>, Error> {
    let running = running_vms();
    Ok(project::projects(root)?
        .into_iter()
        .map(|project| status_with(project, &running))
        .collect())
}

pub(crate) fn status(project: Project) -> ProjectStatus {
    status_with(project, &running_vms())
}

fn status_with(project: Project, running: &[RunnerState]) -> ProjectStatus {
    let vm = sandbox::vm_for(running, &project.dir);
    let (worktrees, error) = match project.repo.as_deref().map(git::agent_worktrees) {
        None => (Vec::new(), None),
        Some(Ok(worktrees)) => (worktrees, None),
        Some(Err(err)) => (Vec::new(), Some(err.to_string())),
    };
    let restart_needed = vm.is_some() && project.restart_needed();
    ProjectStatus {
        project,
        worktrees,
        vm,
        restart_needed,
        error,
    }
}

/// Create a project for the repository and its first worktree, and set up
/// its host side. With `ai`, that agent kind then tailors the flake to the
/// repository. `launch` starts in the first worktree with the VM.
pub(crate) fn create(
    root: &Path,
    worktrees_root: &Path,
    options: &CreateOptions,
    ai: Option<&str>,
    launch: Option<&Launch>,
    stderr: &mut dyn Write,
) -> Result<(Project, Created), Error> {
    let project = project::create(root, options, stderr)?;
    let worktree = git::create_agent_worktree_record(&options.repo, worktrees_root, stderr)?;
    write_worktrees(&project)?;
    let mut steps = vec![Step::Done(format!(
        "project: {} with worktree {}",
        project.dir.display(),
        worktree.name
    ))];
    if let Some(launch) = launch {
        steps.push(record_launch_step(&project, &worktree.name, launch));
    }
    steps.extend(self::setup(&project));
    if let Some(agent) = ai {
        steps.push(
            match project::run_setup_agent(&project, options.source.as_deref(), agent, stderr) {
                Ok(()) => Step::Done(format!("setup: {agent} tailored the flake")),
                Err(err) => Step::Failed(format!("setup: {err}")),
            },
        );
    }
    Ok((project, Created { worktree, steps }))
}

/// Set up the host side of what the project's flake uses: the agent-proxy
/// client, token and CA, and the backlog folder. Safe to re-run.
pub(crate) fn setup(project: &Project) -> Vec<Step> {
    let flake = match std::fs::read_to_string(project.dir.join("flake.nix")) {
        Ok(flake) => flake,
        Err(err) => return vec![Step::Failed(format!("setup: read flake.nix: {err}"))],
    };
    let rerun = "fix it and run `cli-toolbox agent project setup`";
    let proxy = match setup::proxy(project, &flake) {
        Ok(true) => Step::Done(format!(
            "proxy: client {} with its token and CA",
            project.name
        )),
        Ok(false) => Step::Skipped("proxy: not used by flake.nix".into()),
        Err(err) => Step::Failed(format!("proxy: {err}; {rerun}")),
    };
    let backlog = match setup::backlog(project, &flake) {
        Ok(Backlog::Initialized(dir)) => {
            Step::Done(format!("backlog: initialized {}", dir.display()))
        }
        Ok(Backlog::Exists(dir)) => Step::Done(format!("backlog: {}", dir.display())),
        Ok(Backlog::Disabled) => Step::Skipped("backlog: no vm.backlogDir in flake.nix".into()),
        Err(err) => Step::Failed(format!("backlog: {err}; {rerun}")),
    };
    vec![proxy, backlog]
}

/// Create the next agent worktree of the project's repository. A running VM
/// mounts it after a restart, which `restart` does right away. `launch`
/// starts in it once the VM opens it in herdr.
pub(crate) fn add_worktree(
    project: &Project,
    worktrees_root: &Path,
    restart: bool,
    launch: Option<&Launch>,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Created, Error> {
    // Forget the agents of worktrees removed by hand before the new one can
    // reuse such a name.
    write_worktrees(project)?;
    let worktree = git::create_agent_worktree_record(project.repo()?, worktrees_root, stderr)?;
    write_worktrees(project)?;
    let mut steps = Vec::new();
    if let Some(launch) = launch {
        steps.push(record_launch_step(project, &worktree.name, launch));
    }
    let running = sandbox::vm_for(&running_vms(), &project.dir).is_some();
    match (running, restart) {
        (true, true) => steps.extend(self::restart(project, options, stderr)?),
        (true, false) => steps.push(Step::Skipped(format!(
            "vm: running; restart it to mount {}",
            worktree.name
        ))),
        (false, _) => steps.push(Step::Skipped(format!(
            "vm: stopped; starting it mounts {}",
            worktree.name
        ))),
    }
    Ok(Created { worktree, steps })
}

/// The agents chosen for the project's worktrees, by worktree name.
pub(crate) fn launches(project: &Project) -> BTreeMap<String, Launch> {
    std::fs::read_to_string(project.dir.join(AGENTS_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_launches(project: &Project, launches: &BTreeMap<String, Launch>) -> Result<(), Error> {
    let path = project.dir.join(AGENTS_FILE);
    if launches.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                Err(Error::msg(format!("remove {}: {err}", path.display())))
            }
            _ => Ok(()),
        };
    }
    let json = serde_json::to_string_pretty(launches)
        .map_err(|err| Error::msg(format!("serialize {AGENTS_FILE}: {err}")))?;
    std::fs::write(&path, json + "\n")
        .map_err(|err| Error::msg(format!("write {}: {err}", path.display())))
}

/// Remember `launch` for the worktree `name`, to start with the VM.
fn record_launch_step(project: &Project, name: &str, launch: &Launch) -> Step {
    let mut launches = launches(project);
    launches.insert(name.into(), launch.clone());
    match save_launches(project, &launches) {
        Ok(()) => Step::Done(format!(
            "agent: {} starts in {name} with the VM",
            launch.text()
        )),
        Err(err) => Step::Failed(format!("agent: {err}")),
    }
}

/// Start the project's VM with its current worktrees and open them in herdr.
pub(crate) fn start(
    project: &Project,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    let worktrees = write_worktrees(project)?;
    if project.repo.is_some() && worktrees.is_empty() {
        return Err(Error::msg(format!(
            "project {} has no agent worktrees to mount; add one with `cli-toolbox agent project add {}`",
            project.name, project.name
        )));
    }
    if let Some(vm) = sandbox::vm_for(&running_vms(), &project.dir) {
        let status = status(project.clone());
        let mut steps = vec![Step::Skipped(format!("vm: already {}", status.vm_text()))];
        if options.herdr && herdr::available() {
            steps.push(match sandbox::herdr_add(&project.dir) {
                Ok(()) => Step::Done(format!("herdr: machine {}", vm.name)),
                Err(err) => Step::Failed(format!("herdr: reconnect {}: {err}", vm.name)),
            });
        }
        return Ok(steps);
    }
    writeln!(
        stderr,
        "Starting the VM of {} from {}",
        project.name,
        project.dir.display()
    )?;
    let vm = match sandbox::start(&project.dir) {
        Ok(vm) => vm,
        Err(err) => return Ok(vec![Step::Failed(format!("vm: {err}"))]),
    };
    project.record_mounted()?;
    let mut steps = vec![Step::Done(format!(
        "vm: running{} with {} worktree(s)",
        port_text(&vm),
        worktrees.len()
    ))];
    if options.herdr {
        steps.extend(herdr_steps(
            project,
            &vm.name,
            &worktrees,
            options.agent.as_deref(),
        ));
    }
    Ok(steps)
}

/// Stop the project's VM and remove its machine from the host herdr;
/// worktrees and the guest home stay. The next start adds the machine again.
/// Works from `runner.json` rather than the flake's `stop` app: without
/// worktrees, as after the last `done`, the flake does not evaluate.
pub(crate) fn stop(project: &Project) -> Result<Vec<Step>, Error> {
    let Some(vm) = sandbox::vm_for(&running_vms(), &project.dir) else {
        return Ok(vec![Step::Skipped("vm: not running".into())]);
    };
    let stopped = sandbox::stop_vm(&vm, sandbox::stop_timeout())?;
    let mut steps = vec![stopped_step(&vm, stopped)];
    steps.extend(remove_herdr_machine(&vm.name));
    Ok(steps)
}

fn stopped_step(vm: &RunnerState, stopped: sandbox::Stopped) -> Step {
    match stopped {
        sandbox::Stopped::Terminated => Step::Done(format!("vm: stopped {}", vm.name)),
        sandbox::Stopped::Killed => {
            Step::Done(format!("vm: killed {} after it did not stop", vm.name))
        }
    }
}

fn remove_herdr_machine(machine: &str) -> Option<Step> {
    herdr::available().then(|| match herdr::remove_machine(machine) {
        Ok(true) => Step::Done(format!("herdr: removed machine {machine}")),
        Ok(false) => Step::Skipped(format!("herdr: machine {machine} not saved")),
        Err(err) => Step::Failed(format!("herdr: remove machine {machine}: {err}")),
    })
}

/// `agent vm list`: one line per running VM, flagging those whose start
/// directory is gone.
pub(crate) fn vm_list(stdout: &mut dyn Write) -> Result<(), Error> {
    for vm in running_vms() {
        writeln!(stdout, "{}", vm_line(&vm))?;
    }
    Ok(())
}

fn vm_line(vm: &RunnerState) -> String {
    let port = vm
        .ssh_port
        .map(|port| format!("ssh:{port}"))
        .unwrap_or_else(|| "no ssh".into());
    let started = vm.started_at.as_deref().unwrap_or("-");
    let gone = if vm.dir.exists() {
        ""
    } else {
        " (directory gone)"
    };
    format!(
        "{}\tpid {}\t{port}\t{started}\t{}{gone}",
        vm.name,
        vm.pid,
        vm.dir.display()
    )
}

/// The running VMs to stop: every one with `all`, the one named by its host
/// alias or start directory name, the only one, or one picked.
pub(crate) fn select_vms(name: Option<&str>, all: bool) -> Result<Vec<RunnerState>, Error> {
    pick_vms(running_vms(), name, all, |lines| {
        select::pick("vm> ", lines, "40%")
    })
}

fn pick_vms(
    running: Vec<RunnerState>,
    name: Option<&str>,
    all: bool,
    pick: impl FnOnce(&[String]) -> Result<Option<usize>, Error>,
) -> Result<Vec<RunnerState>, Error> {
    if running.is_empty() {
        return Err(Error::msg("no agent-sandbox VM is running"));
    }
    if all {
        return Ok(running);
    }
    if let Some(name) = name {
        return running
            .into_iter()
            .find(|vm| vm.name == name || vm.dir.file_name().is_some_and(|dir| dir == name))
            .map(|vm| vec![vm])
            .ok_or_else(|| Error::msg(format!("no running VM named {name}")));
    }
    if running.len() == 1 {
        return Ok(running);
    }
    let lines: Vec<String> = running.iter().map(vm_line).collect();
    match pick(&lines)? {
        Some(index) => Ok(vec![running[index].clone()]),
        None => Ok(Vec::new()),
    }
}

/// Stop `vm` through its runner and remove its machine from the host herdr.
pub(crate) fn vm_stop(vm: &RunnerState) -> Vec<Step> {
    let mut steps = vec![match sandbox::stop_vm(vm, sandbox::stop_timeout()) {
        Ok(stopped) => stopped_step(vm, stopped),
        Err(err) => return vec![Step::Failed(format!("vm: stop {}: {err}", vm.name))],
    }];
    steps.extend(remove_herdr_machine(&vm.name));
    steps
}

/// `agent vm prune`: remove the runtime directories of stopped VMs whose
/// start directory is gone (logs, SSH config and known hosts).
pub(crate) fn vm_prune(projects_root: &Path) -> Result<Vec<Step>, Error> {
    let Some(root) = sandbox::runtime_root() else {
        return Ok(Vec::new());
    };
    let keep: Vec<_> = project::projects(projects_root)?
        .into_iter()
        .map(|project| project.dir)
        .collect();
    let steps: Vec<Step> = sandbox::stale_runtime_dirs(&root, &keep)
        .into_iter()
        .map(|dir| match std::fs::remove_dir_all(&dir) {
            Ok(()) => Step::Done(format!("removed {}", dir.display())),
            Err(err) => Step::Failed(format!("remove {}: {err}", dir.display())),
        })
        .collect();
    if steps.is_empty() {
        return Ok(vec![Step::Skipped("nothing to prune".into())]);
    }
    Ok(steps)
}

/// Stop and start the VM, so it mounts the current worktrees.
pub(crate) fn restart(
    project: &Project,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    let mut steps = stop(project)?;
    steps.retain(|step| !matches!(step, Step::Skipped(_)));
    steps.extend(start(project, options, stderr)?);
    Ok(steps)
}

/// The result of bringing the agents' commits to the host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Synced {
    /// What git-pull reported, one line per worktree.
    pub(crate) report: String,
    /// Per branch, guest commits that the host branch does not have.
    pub(crate) unsynced: Vec<(String, String)>,
}

/// Fetch the guest's commits and fast-forward the worktrees' branches: the
/// one given, or all of them.
pub(crate) fn sync(project: &Project, worktree: Option<&AgentWorktree>) -> Result<Synced, Error> {
    if sandbox::vm_for(&running_vms(), &project.dir).is_none() {
        return Err(Error::msg(format!(
            "the VM of {} is not running; start it with `cli-toolbox agent project start {}`",
            project.name, project.name
        )));
    }
    let worktrees = match worktree {
        Some(worktree) => vec![worktree.clone()],
        None => git::agent_worktrees(project.repo()?)?,
    };
    // The guest has no Git directory for worktrees added since it started.
    let (mounted, unmounted): (Vec<_>, Vec<_>) = worktrees
        .iter()
        .partition(|worktree| project.mounts(&worktree.name));
    let mut report: String = unmounted
        .iter()
        .map(|worktree| {
            format!(
                "{}: not mounted by the VM yet; restart it to sync.\n",
                worktree.name
            )
        })
        .collect();
    if !mounted.is_empty() {
        let names: Vec<&str> = mounted
            .iter()
            .map(|worktree| worktree.name.as_str())
            .collect();
        report.push_str(&sandbox::git_pull(&project.dir, &names)?);
    }
    let mut unsynced = Vec::new();
    for worktree in &worktrees {
        if let Some(log) = unsynced_commits(&worktree.path, &worktree.branch)? {
            unsynced.push((worktree.branch.clone(), log));
        }
    }
    Ok(Synced { report, unsynced })
}

/// Write `synced` for people: the report, then diverged branches.
pub(crate) fn write_synced(synced: &Synced, out: &mut dyn Write) -> Result<(), Error> {
    write!(out, "{}", synced.report)?;
    for (branch, log) in &synced.unsynced {
        writeln!(
            out,
            "{branch} has diverged from the guest; its commits are in sandbox/{branch}:\n{log}"
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

/// The project's agent worktree named `name` (or its only one).
pub(crate) fn select_worktree(project: &Project, name: &str) -> Result<AgentWorktree, Error> {
    let mut selected = git::select_agent_worktrees(project.repo()?, name, false)?;
    Ok(selected.remove(0))
}

/// Sync a worktree, then remove it and keep its branch for review. Refuses
/// to lose work unless forced. Stops the VM when it was the last worktree.
pub(crate) fn done(
    project: &Project,
    worktree: &AgentWorktree,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    let vm = sandbox::vm_for(&running_vms(), &project.dir);
    if let Some(problem) = done_blocker(project, worktree, vm.is_some(), options.force)? {
        return Err(Error::msg(format!(
            "{problem}\nPass --force to remove {} anyway; its branch {} stays.",
            worktree.name, worktree.branch
        )));
    }
    let remove = git::RemoveOptions {
        keep_branch: true,
        // Uncommitted and untracked files were checked above.
        force: true,
    };
    git::remove_agent_worktree(project.repo()?, worktree, remove, stderr)?;
    let mut steps = vec![Step::Done(format!(
        "worktree: removed {}; branch {} kept for review",
        worktree.name, worktree.branch
    ))];
    let remaining = write_worktrees(project)?;
    if vm.is_some() {
        if remaining.is_empty() {
            steps.extend(stop(project)?);
        } else {
            steps.push(Step::Skipped(
                "vm: still mounts it until the next restart".into(),
            ));
        }
    }
    Ok(steps)
}

/// Why `done` would lose work, if it would.
fn done_blocker(
    project: &Project,
    worktree: &AgentWorktree,
    running: bool,
    force: bool,
) -> Result<Option<String>, Error> {
    if force {
        return Ok(None);
    }
    // A worktree added after the VM started has no guest commits to lose.
    if running && project.mounts(&worktree.name) {
        sandbox::git_pull(&project.dir, &[&worktree.name])?;
        if let Some(unsynced) = unsynced_commits(&worktree.path, &worktree.branch)? {
            return Ok(Some(format!(
                "{} has diverged from the guest; these guest commits would be lost:\n{unsynced}",
                worktree.branch
            )));
        }
    } else if project.state_dir.join("home.img").exists() {
        return Ok(Some(format!(
            "the VM of {} is stopped, so the latest commits of {} cannot be synced; run `cli-toolbox agent project start {}` first",
            project.name, worktree.name, project.name
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

/// `agent project list`: one line per project, then its worktrees.
pub(crate) fn list(root: &Path, stdout: &mut dyn Write) -> Result<(), Error> {
    for status in statuses(root)? {
        write!(stdout, "{}", list_lines(&status))?;
    }
    Ok(())
}

fn list_lines(status: &ProjectStatus) -> String {
    let repo = status
        .project
        .repo
        .as_ref()
        .map(|repo| repo.display().to_string())
        .unwrap_or_else(|| "(no repository)".into());
    let mut text = format!("{}\t{}\t{repo}\n", status.project.name, status.vm_text());
    if let Some(error) = &status.error {
        text.push_str(&format!("  error: {error}\n"));
    }
    for worktree in &status.worktrees {
        text.push_str(&format!(
            "  {}\t{}\t{}\n",
            worktree.name,
            worktree.branch,
            worktree.path.display()
        ));
    }
    text
}

/// Write `worktrees.nix` from the repository's agent worktrees.
fn write_worktrees(project: &Project) -> Result<Vec<AgentWorktree>, Error> {
    let Some(repo) = project.repo.as_deref() else {
        return Ok(Vec::new());
    };
    let worktrees = git::agent_worktrees(repo)?;
    project.write_worktrees(&worktrees)?;
    // A later worktree of the same name must not inherit a removed one's agent.
    let mut launches = launches(project);
    let count = launches.len();
    launches.retain(|name, _| worktrees.iter().any(|worktree| worktree.name == *name));
    if launches.len() != count {
        save_launches(project, &launches)?;
    }
    Ok(worktrees)
}

/// Add the VM to herdr and open a workspace per worktree, starting the
/// worktree's own agent or else `agent` in it. A worktree's first prompt is
/// sent once.
fn herdr_steps(
    project: &Project,
    machine: &str,
    worktrees: &[AgentWorktree],
    agent: Option<&str>,
) -> Vec<Step> {
    if !herdr::available() {
        return vec![Step::Skipped("herdr: not installed".into())];
    }
    if let Err(err) = sandbox::herdr_add(&project.dir) {
        return vec![Step::Failed(format!("herdr: add machine {machine}: {err}"))];
    }
    let mut steps = vec![Step::Done(format!("herdr: machine {machine}"))];
    let mut launches = launches(project);
    let mut prompted = false;
    for worktree in worktrees {
        let cwd = guest_dir(&worktree.name);
        let opened = match herdr::open_machine_workspace(machine, &cwd, &worktree.name) {
            Ok(opened) => opened,
            Err(err) => {
                steps.push(Step::Failed(format!("herdr: open workspace {cwd}: {err}")));
                continue;
            }
        };
        let id = &opened.workspace_id;
        let launch = launches
            .get(&worktree.name)
            .cloned()
            .or_else(|| agent.map(|kind| Launch::new(kind, None)));
        let Some(launch) = launch else {
            steps.push(Step::Done(format!("herdr: workspace {id} at {cwd}")));
            continue;
        };
        steps.push(
            match super::start_agent(Some(machine), &worktree.name, &opened.pane_id, &launch) {
                Ok(running) => {
                    if let Some(own) = launches.get_mut(&worktree.name) {
                        prompted |= own.prompt.take().is_some();
                    }
                    Step::Done(format!("herdr: workspace {id} at {cwd} running {running}"))
                }
                Err(err) => Step::Failed(format!(
                    "herdr: workspace {id} at {cwd}, but could not start {}: {err}",
                    launch.kind
                )),
            },
        );
    }
    if prompted && let Err(err) = save_launches(project, &launches) {
        steps.push(Step::Failed(format!("agent: forget sent prompts: {err}")));
    }
    steps
}

/// Where a worktree is mounted in the guest.
pub(crate) fn guest_dir(worktree: &str) -> String {
    format!("/workspace/{worktree}")
}

fn port_text(vm: &RunnerState) -> String {
    vm.ssh_port
        .map(|port| format!(" on SSH port {port}"))
        .unwrap_or_default()
}

/// Every running agent-sandbox VM.
pub(crate) fn running_vms() -> Vec<RunnerState> {
    sandbox::runtime_root()
        .map(|root| sandbox::running_vms(&root))
        .unwrap_or_default()
}

/// Run `each` on every project whose VM is running, for the `--all`
/// commands. Steps name their project; an error fails that project only, so
/// the others still run.
pub(crate) fn each_running(
    root: &Path,
    mut each: impl FnMut(&Project) -> Result<Vec<Step>, Error>,
) -> Result<Vec<Step>, Error> {
    let mut steps = Vec::new();
    for project in running_projects(root)? {
        let done = each(&project).unwrap_or_else(|err| vec![Step::Failed(err.to_string())]);
        steps.extend(done.into_iter().map(|step| step.prefixed(&project.name)));
    }
    Ok(steps)
}

/// Sync every project whose VM is running, writing each report to `out`
/// under a `<project>:` heading.
pub(crate) fn sync_all(root: &Path, out: &mut dyn Write) -> Result<Vec<Step>, Error> {
    each_running(root, |project| {
        let synced = sync(project, None)?;
        writeln!(out, "{}:", project.name)?;
        write_synced(&synced, out)?;
        Ok(vec![Step::Done("synced".into())])
    })
}

/// The projects in `root` whose VM is running, for the `--all` commands.
pub(crate) fn running_projects(root: &Path) -> Result<Vec<Project>, Error> {
    with_running_vm(project::projects(root)?, &running_vms())
}

fn with_running_vm(projects: Vec<Project>, running: &[RunnerState]) -> Result<Vec<Project>, Error> {
    let projects: Vec<Project> = projects
        .into_iter()
        .filter(|project| sandbox::vm_for(running, &project.dir).is_some())
        .collect();
    if projects.is_empty() {
        return Err(Error::msg("no sandbox project VM is running"));
    }
    Ok(projects)
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

    /// A project for `repo` in `root`, as `project create` leaves it.
    fn project(root: &Path, repo: &Path) -> Project {
        let dir = root.join("projects/demo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("flake.nix"), "{ }").unwrap();
        std::fs::write(
            dir.join("project.json"),
            format!("{{\"repo\": \"{}\"}}", repo.display()),
        )
        .unwrap();
        project::load(root, "demo").unwrap()
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
    fn added_worktrees_are_mounted_by_the_project() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let project = project(&temp.path().join("sandbox"), &repo);
        let worktrees_root = temp.path().join("worktrees");
        let options = VmOptions::default();

        let first = add_worktree(
            &project,
            &worktrees_root,
            false,
            None,
            &options,
            &mut Vec::new(),
        )
        .unwrap();
        let second = add_worktree(
            &project,
            &worktrees_root,
            false,
            None,
            &options,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(second.worktree.name, "demo-agent-wt-2");
        assert!(matches!(&first.steps[..], [Step::Skipped(text)] if text.contains("stopped")));

        let mounted = std::fs::read_to_string(project.dir.join("worktrees.nix")).unwrap();
        assert!(mounted.contains("\"demo-agent-wt\" = "), "{mounted}");
        assert!(mounted.contains("\"demo-agent-wt-2\" = "), "{mounted}");

        let status = status(project.clone());
        assert_eq!(status.worktrees.len(), 2);
        assert_eq!(status.vm, None);
        assert_eq!(status.vm_text(), "stopped");
    }

    #[test]
    fn worktree_agents_are_kept_until_the_worktree_is_gone() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let project = project(&temp.path().join("sandbox"), &repo);
        let worktrees_root = temp.path().join("worktrees");
        let launch = Launch::new("codex", Some("Add a --json flag".into()));

        let created = add_worktree(
            &project,
            &worktrees_root,
            false,
            Some(&launch),
            &VmOptions::default(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            created.steps[0],
            Step::Done("agent: codex with the prompt starts in demo-agent-wt with the VM".into())
        );
        assert_eq!(
            launches(&project),
            BTreeMap::from([("demo-agent-wt".to_string(), launch)])
        );

        // Removed some other way: the next worktree reuses its name.
        git::remove_agent_worktree(
            &repo,
            &created.worktree,
            git::RemoveOptions::default(),
            &mut Vec::new(),
        )
        .unwrap();
        let again = add_worktree(
            &project,
            &worktrees_root,
            false,
            None,
            &VmOptions::default(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(again.worktree.name, "demo-agent-wt");
        assert!(launches(&project).is_empty());
        assert!(!project.dir.join(AGENTS_FILE).exists());
    }

    #[test]
    fn starting_needs_a_worktree() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let project = project(&temp.path().join("sandbox"), &repo);
        let err = start(&project, &VmOptions::default(), &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("no agent worktrees"), "{err}");
    }

    #[test]
    fn done_refuses_uncommitted_changes_and_stopped_vms_with_a_home() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let project = project(&temp.path().join("sandbox"), &repo);
        let created = add_worktree(
            &project,
            &temp.path().join("worktrees"),
            false,
            None,
            &VmOptions::default(),
            &mut Vec::new(),
        )
        .unwrap();
        let worktree = created.worktree;
        assert_eq!(
            done_blocker(&project, &worktree, false, false).unwrap(),
            None
        );

        std::fs::write(worktree.path.join("wip.txt"), "wip\n").unwrap();
        let blocker = done_blocker(&project, &worktree, false, false)
            .unwrap()
            .unwrap();
        assert!(blocker.contains("?? wip.txt"), "{blocker}");
        std::fs::remove_file(worktree.path.join("wip.txt")).unwrap();

        // Added after the VM started: the guest never had it to pull from.
        std::fs::create_dir_all(&project.state_dir).unwrap();
        std::fs::write(project.state_dir.join("mounted-worktrees.nix"), "{\n}\n").unwrap();
        assert_eq!(
            done_blocker(&project, &worktree, true, false).unwrap(),
            None
        );
        std::fs::write(worktree.path.join("wip.txt"), "wip\n").unwrap();

        std::fs::create_dir_all(&project.state_dir).unwrap();
        std::fs::write(project.state_dir.join("home.img"), "home").unwrap();
        let blocker = done_blocker(&project, &worktree, false, false)
            .unwrap()
            .unwrap();
        assert!(blocker.contains("is stopped"), "{blocker}");
        assert_eq!(
            done_blocker(&project, &worktree, false, true).unwrap(),
            None
        );

        let forced = VmOptions {
            force: true,
            ..VmOptions::default()
        };
        done(&project, &worktree, &forced, &mut Vec::new()).unwrap();
        assert!(git::agent_worktrees(&repo).unwrap().is_empty());
        let mounted = std::fs::read_to_string(project.dir.join("worktrees.nix")).unwrap();
        assert!(!mounted.contains("demo-agent-wt"), "{mounted}");
    }

    #[test]
    fn list_shows_the_vm_and_worktrees_of_each_project() {
        let status = ProjectStatus {
            project: Project {
                name: "demo".into(),
                dir: "/sandbox/projects/demo".into(),
                repo: Some("/p/demo".into()),
                state_dir: "/sandbox/state/demo".into(),
            },
            worktrees: vec![AgentWorktree {
                name: "demo-agent-wt".into(),
                path: "/wt/demo-agent-wt".into(),
                branch: "demo-agent-wt".into(),
            }],
            vm: Some(RunnerState {
                pid: 1,
                ssh_port: Some(2301),
                name: "agent-demo".into(),
                dir: "/sandbox/projects/demo".into(),
                runtime_dir: "/state".into(),
                ..RunnerState::default()
            }),
            restart_needed: false,
            error: None,
        };
        assert_eq!(
            list_lines(&status),
            "demo\trunning on SSH port 2301\t/p/demo\n  demo-agent-wt\tdemo-agent-wt\t/wt/demo-agent-wt\n"
        );
        let stale = ProjectStatus {
            restart_needed: true,
            ..status
        };
        assert!(stale.vm_text().contains("restart"));
    }

    fn running(names: &[&str]) -> Vec<RunnerState> {
        names
            .iter()
            .enumerate()
            .map(|(pid, name)| RunnerState {
                pid: pid as u32 + 100,
                ssh_port: Some(2300),
                name: format!("agent-{name}"),
                dir: format!("/gone/projects/{name}").into(),
                started_at: Some("2026-10-21T11:21:01Z".into()),
                ..RunnerState::default()
            })
            .collect()
    }

    #[test]
    fn steps_of_several_projects_name_their_project() {
        assert_eq!(
            Step::Failed("vm: boom".into()).prefixed("demo"),
            Step::Failed("demo: vm: boom".into())
        );
    }

    #[test]
    fn all_takes_the_projects_with_a_running_vm() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let projects: Vec<Project> = ["a", "b", "c"]
            .iter()
            .map(|name| Project {
                name: (*name).into(),
                dir: root.join("projects").join(name),
                repo: None,
                state_dir: root.join("state").join(name),
            })
            .collect();
        let vms = running(&["a", "c", "gone"])
            .into_iter()
            .map(|vm| RunnerState {
                dir: root
                    .join("projects")
                    .join(vm.name.trim_start_matches("agent-")),
                ..vm
            })
            .collect::<Vec<_>>();

        let names = |projects: Vec<Project>| {
            projects
                .into_iter()
                .map(|project| project.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(with_running_vm(projects.clone(), &vms).unwrap()),
            ["a", "c"]
        );
        let err = with_running_vm(projects, &[]).unwrap_err();
        assert!(err.to_string().contains("no sandbox project VM"), "{err}");
    }

    #[test]
    fn vm_lines_flag_missing_directories() {
        assert_eq!(
            vm_line(&running(&["demo"])[0]),
            "agent-demo\tpid 100\tssh:2300\t2026-10-21T11:21:01Z\t/gone/projects/demo (directory gone)"
        );
    }

    #[test]
    fn vms_are_picked_by_name_alone_or_from_a_list() {
        let unused = |_: &[String]| -> Result<Option<usize>, Error> { panic!("no pick") };
        let names = |vms: Vec<RunnerState>| vms.into_iter().map(|vm| vm.name).collect::<Vec<_>>();

        let picked = pick_vms(running(&["a", "b"]), Some("b"), false, unused).unwrap();
        assert_eq!(names(picked), ["agent-b"]);
        let picked = pick_vms(running(&["a", "b"]), Some("agent-a"), false, unused).unwrap();
        assert_eq!(names(picked), ["agent-a"]);
        let picked = pick_vms(running(&["a"]), None, false, unused).unwrap();
        assert_eq!(names(picked), ["agent-a"]);
        let picked = pick_vms(running(&["a", "b"]), None, true, unused).unwrap();
        assert_eq!(names(picked), ["agent-a", "agent-b"]);
        let picked = pick_vms(running(&["a", "b"]), None, false, |lines| {
            assert_eq!(lines.len(), 2);
            Ok(Some(1))
        })
        .unwrap();
        assert_eq!(names(picked), ["agent-b"]);
        assert!(pick_vms(running(&["a"]), Some("c"), false, unused).is_err());
        assert!(pick_vms(Vec::new(), None, false, unused).is_err());
    }
}
