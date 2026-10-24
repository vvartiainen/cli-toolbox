//! Sandbox projects: one agent-sandbox microVM per project, defined in the
//! central sandbox directory and mounting every agent worktree of the
//! project's repository at `/workspace/<worktree>`. The VM is added to the
//! host herdr as a machine, with a guest workspace per worktree. The agents'
//! work is published as host snapshot branches; host history is made available
//! to the agent separately, without moving either side's active branch.
//!
//! The `vm_*` functions manage every running agent-sandbox VM from its
//! runner state alone, also when its project directory is gone.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use super::prune::{self, PruneOptions};
use super::{Created, Launch, Step};
use crate::git::{self, Worktree};
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
    /// `done`: keep the worktree's branch even when its work is accepted.
    pub(crate) keep_branch: bool,
}

/// A project with its worktrees and VM, for lists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectStatus {
    pub(crate) project: Project,
    pub(crate) worktrees: Vec<Worktree>,
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
    let listed = project.repo.is_some().then(|| project.worktrees());
    let (worktrees, error) = match listed {
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

/// Create a project for the repository and set up its host side, as
/// `options` say. With `ai`, that agent kind then tailors the flake to the
/// repository. The project has no worktree yet; `add_worktree` adds them.
pub(crate) fn create(
    root: &Path,
    options: &CreateOptions,
    ai: Option<&str>,
    stderr: &mut dyn Write,
) -> Result<(Project, Vec<Step>), Error> {
    let project = project::create(root, options, stderr)?;
    // Worktrees left in the repository's group, from an earlier project, count.
    let worktrees = write_worktrees(&project)?;
    let mut steps = vec![Step::Done(format!("project: {}", project.dir.display()))];
    steps.extend(self::setup(&project));
    if let Some(agent) = ai {
        steps.push(
            match project::run_setup_agent(&project, options.source.as_deref(), agent, stderr) {
                Ok(()) => Step::Done(format!("setup: {agent} tailored the flake")),
                Err(err) => Step::Failed(format!("setup: {err}")),
            },
        );
    }
    if worktrees.is_empty() {
        steps.push(Step::Skipped(format!(
            "worktree: none yet; add one with `cli-toolbox agent project add {} --name NAME`",
            project.name
        )));
    }
    Ok((project, steps))
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

/// Create the worktree `name` of the project's repository (see
/// [`git::create_worktree`]), or else the next `<repo>-agent-wt-N`. A running
/// VM mounts it after a restart, which `restart` does right away. `launch`
/// starts in it once the VM opens it in herdr; it must be an agent the
/// project enables, when its flake names any.
pub(crate) fn add_worktree(
    project: &Project,
    name: Option<&str>,
    restart: bool,
    launch: Option<&Launch>,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Created, Error> {
    // Forget the agents of worktrees removed by hand before the new one can
    // reuse such a name.
    if let Some(launch) = launch {
        let enabled = enabled_agents(project)?;
        if !enabled.is_empty() && !enabled.contains(&launch.kind) {
            return Err(Error::msg(format!(
                "project {} does not enable {}; its flake enables {}",
                project.name,
                launch.kind,
                enabled.join(", ")
            )));
        }
    }
    write_worktrees(project)?;
    let name = match name {
        Some(name) => name.to_string(),
        None => suggested_worktree_name(project)?,
    };
    let worktree = git::create_worktree(project.repo()?, &project.worktrees_root, &name, stderr)?;
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

/// Update the inputs of the project's flake, such as agent-sandbox. A
/// running VM uses them after a restart.
pub(crate) fn update_flake(project: &Project) -> Result<Vec<Step>, Error> {
    project::update_flake(project)?;
    let mut steps = vec![Step::Done(format!(
        "flake: updated {}",
        project.dir.join("flake.lock").display()
    ))];
    if sandbox::vm_for(&running_vms(), &project.dir).is_some() {
        steps.push(Step::Skipped(
            "vm: running; restart it to use the update".into(),
        ));
    }
    Ok(steps)
}

/// The name a new worktree of the project gets unless named otherwise:
/// the next free `<repo>-agent-wt-N`.
pub(crate) fn suggested_worktree_name(project: &Project) -> Result<String, Error> {
    git::next_agent_worktree_name(project.repo()?, &project.worktrees_root)
}

/// The coding agents the project's flake enables (`ai.<kind>.enable`), the
/// ones a worktree of it can start.
pub(crate) fn enabled_agents(project: &Project) -> Result<Vec<String>, Error> {
    let path = project.dir.join("flake.nix");
    let flake = std::fs::read_to_string(&path)
        .map_err(|err| Error::msg(format!("read {}: {err}", path.display())))?;
    Ok(setup::enabled_agents(&flake))
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
/// worktrees and the guest home stay, but its Herdr session is reset before
/// shutdown. The next start adds the machine again with fresh workspaces.
/// Works from `runner.json` rather than the flake's `stop` app: without
/// worktrees, as after the last `done`, the flake does not evaluate.
pub(crate) fn stop(project: &Project) -> Result<Vec<Step>, Error> {
    let Some(vm) = sandbox::vm_for(&running_vms(), &project.dir) else {
        return Ok(vec![Step::Skipped("vm: not running".into())]);
    };
    Ok(vm_stop(&vm))
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
    vm_stop_with(
        vm,
        || remove_herdr_machine(&vm.name),
        || sandbox::stop_vm(vm, sandbox::stop_timeout()),
    )
}

fn vm_stop_with(
    vm: &RunnerState,
    cleanup: impl FnOnce() -> Option<Step>,
    shutdown: impl FnOnce() -> Result<sandbox::Stopped, Error>,
) -> Vec<Step> {
    // The guest must still be reachable to discard its persistent session.
    // Cleanup failure must not prevent shutdown or hide either result.
    let mut steps: Vec<_> = cleanup().into_iter().collect();
    steps.push(match shutdown() {
        Ok(stopped) => stopped_step(vm, stopped),
        Err(err) => Step::Failed(format!("vm: stop {}: {err}", vm.name)),
    });
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

/// History exchange results. Divergence needs review, not a failed sync.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Synced {
    pub(crate) report: String,
    pub(crate) failures: Vec<String>,
}

/// Publish agent history as host review snapshots and send normal host history
/// to the guest's review refs. Never advance an existing branch or touch files.
pub(crate) fn sync(project: &Project, worktree: Option<&Worktree>) -> Result<Synced, Error> {
    let vm = sandbox::vm_for(&running_vms(), &project.dir).ok_or_else(|| {
        Error::msg(format!(
            "the VM of {} is not running; start it with `cli-toolbox agent project start {}`",
            project.name, project.name
        ))
    })?;
    let worktrees = match worktree {
        Some(worktree) => vec![worktree.clone()],
        None => project.worktrees()?,
    };
    Ok(sync_worktrees(
        &worktrees,
        |name| project.mounts(name),
        |worktree| {
            sandbox::sync::Bridge::for_vm(&vm, &worktree.name)?
                .exchange(&worktree.path, &worktree.branch)
        },
    ))
}

fn sync_worktrees(
    worktrees: &[Worktree],
    mounted: impl Fn(&str) -> bool,
    mut exchange: impl FnMut(&Worktree) -> Result<String, Error>,
) -> Synced {
    let mut synced = Synced {
        report: String::new(),
        failures: Vec::new(),
    };
    for worktree in worktrees {
        if !mounted(&worktree.name) {
            synced.report.push_str(&format!(
                "{}: not mounted by the VM yet; restart it to sync.\n",
                worktree.name
            ));
            continue;
        }
        synced.report.push_str(&format!("{}:\n", worktree.name));
        match exchange(worktree) {
            Ok(report) => synced.report.push_str(&report),
            Err(err) => {
                synced
                    .report
                    .push_str(&format!("  history exchange failed: {err}\n"));
                synced.failures.push(worktree.name.clone());
            }
        }
    }
    synced
}

pub(crate) fn write_synced(synced: &Synced, out: &mut dyn Write) -> Result<(), Error> {
    write!(out, "{}", synced.report)?;
    if !synced.failures.is_empty() {
        return Err(Error::msg(format!(
            "history exchange failed for {}",
            synced.failures.join(", ")
        )));
    }
    Ok(())
}

/// The project's agent worktree named `name` (or its only one).
pub(crate) fn select_worktree(project: &Project, name: &str) -> Result<Worktree, Error> {
    let mut selected = git::select_worktrees(project.worktrees()?, name, false)?;
    Ok(selected.remove(0))
}

/// Save a worktree's committed guest work as review snapshots, then remove
/// it. Refuses to lose work unless forced. Its branch is deleted when its
/// commits are already on the integration branch, and kept otherwise or with
/// `keep_branch`. Stops the VM when it was the last worktree. Then prunes the
/// worktree's accepted snapshots; a failed prune does not fail it, since the
/// worktree is gone by then.
pub(crate) fn done(
    project: &Project,
    worktree: &Worktree,
    options: &VmOptions,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    let vm = sandbox::vm_for(&running_vms(), &project.dir);
    if let Some(problem) = done_blocker(project, worktree, vm.as_ref(), options.force, stderr)? {
        return Err(Error::msg(format!(
            "{problem}\nPass --force to remove {} anyway.",
            worktree.name
        )));
    }
    let kept = if options.keep_branch {
        Some("--keep-branch".to_string())
    } else {
        unaccepted(project.repo()?, &worktree.branch)
    };
    let (mut steps, remaining) = remove(project, worktree, kept.is_some(), stderr)?;
    if let Some(reason) = kept.filter(|_| !worktree.branch.is_empty()) {
        steps.push(Step::Skipped(format!(
            "branch {} kept: {reason}",
            worktree.branch
        )));
    }
    let options = PruneOptions {
        worktree: Some(worktree.name.clone()),
        ..PruneOptions::default()
    };
    match prune::prune(project, &options) {
        Ok(report) => steps.extend(report.steps.into_iter().map(|step| match step {
            Step::Failed(text) => Step::Skipped(format!("prune failed: {text}")),
            step => step,
        })),
        Err(err) => steps.push(Step::Skipped(format!("prune: {err}"))),
    }
    if vm.is_some() {
        if remaining.is_empty() {
            steps.extend(stop(project)?);
        } else {
            steps.push(still_mounted());
        }
    }
    Ok(steps)
}

/// Why `branch` must stay, if it must: commits whose changes the
/// integration branch lacks, or no integration branch to compare with.
fn unaccepted(repo: &Path, branch: &str) -> Option<String> {
    if branch.is_empty() {
        return None;
    }
    let into = match git::integration_branch(repo) {
        Ok(into) => into,
        Err(err) => return Some(err.to_string()),
    };
    if into == branch {
        return Some("it is the integration branch".into());
    }
    match git::unapplied_commits(repo, &into, branch) {
        Ok(commits) if commits.is_empty() => None,
        Ok(commits) => Some(format!(
            "{} commit{} not on {into}",
            commits.len(),
            if commits.len() == 1 { "" } else { "s" }
        )),
        Err(err) => Some(format!("compare with {into}: {err}")),
    }
}

/// Remove the project's worktree without saving its work first: uncommitted
/// changes and guest commits that no sync saved are lost. Saved review
/// snapshots stay. Its branch is deleted unless `keep_branch`. Works with the
/// VM stopped; a running VM mounts it until its next restart.
pub(crate) fn remove_worktree(
    project: &Project,
    worktree: &Worktree,
    keep_branch: bool,
    stderr: &mut dyn Write,
) -> Result<Vec<Step>, Error> {
    let (mut steps, _) = remove(project, worktree, keep_branch, stderr)?;
    if sandbox::vm_for(&running_vms(), &project.dir).is_some() {
        steps.push(still_mounted());
    }
    Ok(steps)
}

/// Force-remove the worktree and rewrite `worktrees.nix`. Returns the steps
/// and the worktrees left.
fn remove(
    project: &Project,
    worktree: &Worktree,
    keep_branch: bool,
    stderr: &mut dyn Write,
) -> Result<(Vec<Step>, Vec<Worktree>), Error> {
    let options = git::RemoveOptions {
        keep_branch,
        force: true,
    };
    git::remove_worktree(project.repo()?, worktree, options, stderr)?;
    let removed = match (keep_branch, worktree.branch.as_str()) {
        (_, "") => format!("worktree: removed {}", worktree.name),
        (true, branch) => format!("worktree: removed {}; branch {branch} kept", worktree.name),
        (false, branch) => format!("worktree: removed {} and branch {branch}", worktree.name),
    };
    let remaining = write_worktrees(project)?;
    Ok((vec![Step::Done(removed)], remaining))
}

fn still_mounted() -> Step {
    Step::Skipped("vm: still mounts it until the next restart".into())
}

/// Why `done` would lose work, if it would.
fn done_blocker(
    project: &Project,
    worktree: &Worktree,
    vm: Option<&RunnerState>,
    force: bool,
    report: &mut dyn Write,
) -> Result<Option<String>, Error> {
    if force {
        return Ok(None);
    }
    // A worktree added after the VM started has no guest commits to lose.
    if let Some(vm) = vm.filter(|_| project.mounts(&worktree.name)) {
        let review = sandbox::sync::Bridge::for_vm(vm, &worktree.name)?
            .prepare_done(&worktree.path, &worktree.branch)?;
        write!(report, "{}", review.report)?;
        // The guest's clean index is authoritative. Its committed work is now
        // on ordinary host snapshot branches, independent of this stale index.
        return Ok(None);
    } else if vm.is_none() && project.state_dir.join("home.img").exists() {
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
fn write_worktrees(project: &Project) -> Result<Vec<Worktree>, Error> {
    if project.repo.is_none() {
        return Ok(Vec::new());
    }
    let worktrees = project.worktrees()?;
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
/// worktree's own agent or else `agent`, when the project enables it. A
/// worktree's first prompt is sent once.
fn herdr_steps(
    project: &Project,
    machine: &str,
    worktrees: &[Worktree],
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
    let enabled = enabled_agents(project).unwrap_or_default();
    let agent =
        agent.filter(|kind| enabled.is_empty() || enabled.iter().any(|known| known == kind));
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

/// How many worktrees the project in `dir` mounts, from its `worktrees.nix`.
pub(crate) fn worktree_count(dir: &Path) -> Option<usize> {
    let text = std::fs::read_to_string(dir.join("worktrees.nix")).ok()?;
    Some(text.lines().filter(|line| line.contains(" = ")).count())
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
        Ok(vec![Step::Done("review snapshots published".into())])
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

    #[test]
    fn session_cleanup_precedes_shutdown_even_when_it_fails() {
        let events = std::cell::RefCell::new(Vec::new());
        let vm = RunnerState {
            name: "agent-demo".into(),
            ..Default::default()
        };
        let steps = vm_stop_with(
            &vm,
            || {
                events.borrow_mut().push("cleanup");
                Some(Step::Failed("herdr: SSH unavailable".into()))
            },
            || {
                events.borrow_mut().push("shutdown");
                Ok(sandbox::Stopped::Terminated)
            },
        );
        assert_eq!(*events.borrow(), ["cleanup", "shutdown"]);
        assert!(matches!(&steps[0], Step::Failed(message) if message.contains("SSH unavailable")));
        assert!(matches!(&steps[1], Step::Done(message) if message.contains("stopped")));
    }

    #[test]
    fn shutdown_failure_does_not_hide_the_cleanup_result() {
        let steps = vm_stop_with(
            &RunnerState::default(),
            || Some(Step::Done("herdr: removed machine".into())),
            || Err(Error::msg("runner unavailable")),
        );
        assert!(matches!(&steps[0], Step::Done(_)));
        assert!(
            matches!(&steps[1], Step::Failed(message) if message.contains("runner unavailable"))
        );
    }
    use crate::git::tests::{init_git_repo, skip_without_git};

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
        Project {
            worktrees_root: root.join("worktrees"),
            ..project::load(root, "demo").unwrap()
        }
    }

    #[test]
    fn sync_skips_unmounted_worktrees_and_continues_after_transport_errors() {
        let worktrees: Vec<_> = ["unmounted", "broken", "ready"]
            .into_iter()
            .map(|name| Worktree {
                name: name.into(),
                path: "/unused".into(),
                branch: name.into(),
            })
            .collect();
        let mut called = Vec::new();
        let synced = sync_worktrees(
            &worktrees,
            |name| name != "unmounted",
            |worktree| {
                called.push(worktree.name.clone());
                if worktree.name == "broken" {
                    Err(Error::msg("SSH unavailable"))
                } else {
                    Ok("guest: 1 new patch to review\n".into())
                }
            },
        );
        assert_eq!(called, ["broken", "ready"]);
        assert_eq!(synced.failures, ["broken"]);
        assert!(synced.report.contains("not mounted"));
        assert!(synced.report.contains("SSH unavailable"));
        let mut output = Vec::new();
        assert!(write_synced(&synced, &mut output).is_err());
        assert!(String::from_utf8(output).unwrap().contains("1 new patch"));
    }

    #[test]
    fn review_needed_is_a_successful_sync() {
        let synced = Synced {
            report: "guest: 1 new patch to review\n".into(),
            failures: Vec::new(),
        };
        assert!(write_synced(&synced, &mut Vec::new()).is_ok());
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
        let options = VmOptions::default();

        let named = Some("feature/login");
        let first = add_worktree(&project, named, false, None, &options, &mut Vec::new()).unwrap();
        let second = add_worktree(&project, None, false, None, &options, &mut Vec::new()).unwrap();
        assert_eq!(first.worktree.name, "feature-login");
        assert_eq!(first.worktree.branch, "feature/login");
        assert_eq!(second.worktree.name, "demo-agent-wt");
        assert_eq!(
            suggested_worktree_name(&project).unwrap(),
            "demo-agent-wt-2"
        );
        assert!(matches!(&first.steps[..], [Step::Skipped(text)] if text.contains("stopped")));

        let mounted = std::fs::read_to_string(project.dir.join("worktrees.nix")).unwrap();
        assert!(mounted.contains("\"feature-login\" = "), "{mounted}");
        assert!(mounted.contains("\"demo-agent-wt\" = "), "{mounted}");

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
        let launch = Launch::new("codex", Some("Add a --json flag".into()));

        let created = add_worktree(
            &project,
            None,
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
        git::remove_worktree(
            &repo,
            &created.worktree,
            git::RemoveOptions::default(),
            &mut Vec::new(),
        )
        .unwrap();
        let again = add_worktree(
            &project,
            None,
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
    fn only_agents_the_flake_enables_can_start() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let project = project(&temp.path().join("sandbox"), &repo);
        std::fs::write(
            project.dir.join("flake.nix"),
            "{\n  ai = {\n    claude.enable = true;\n  };\n}\n",
        )
        .unwrap();
        assert_eq!(enabled_agents(&project).unwrap(), ["claude"]);
        let add = |kind: &str| {
            let launch = Launch::new(kind, None);
            add_worktree(
                &project,
                Some(kind),
                false,
                Some(&launch),
                &VmOptions::default(),
                &mut Vec::new(),
            )
        };
        let err = add("codex").err().unwrap();
        assert!(err.to_string().contains("does not enable codex"), "{err}");
        assert!(project.worktrees().unwrap().is_empty());
        assert_eq!(add("claude").unwrap().worktree.name, "claude");
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

    fn branches(repo: &Path) -> Vec<String> {
        crate::git::tests::must_git_output(
            repo,
            &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
        )
        .lines()
        .map(String::from)
        .collect()
    }

    #[test]
    fn worktrees_are_removed_without_saving_also_with_the_vm_stopped() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let project = project(&temp.path().join("sandbox"), &repo);
        // A persistent home would make done refuse; removal does not care.
        std::fs::create_dir_all(&project.state_dir).unwrap();
        std::fs::write(project.state_dir.join("home.img"), "home").unwrap();
        let add = |name: &str| {
            add_worktree(
                &project,
                Some(name),
                false,
                None,
                &VmOptions::default(),
                &mut Vec::new(),
            )
            .unwrap()
            .worktree
        };
        let kept = add("kept");
        let gone = add("gone");
        std::fs::write(kept.path.join("wip.txt"), "lost\n").unwrap();

        let steps = remove_worktree(&project, &kept, true, &mut Vec::new()).unwrap();
        assert_eq!(
            steps,
            [Step::Done(
                "worktree: removed kept; branch kept kept".into()
            )]
        );
        remove_worktree(&project, &gone, false, &mut Vec::new()).unwrap();
        assert!(project.worktrees().unwrap().is_empty());
        assert!(!kept.path.exists());
        let branches = branches(&repo);
        assert!(branches.contains(&"kept".to_string()), "{branches:?}");
        assert!(!branches.contains(&"gone".to_string()), "{branches:?}");
        let mounted = std::fs::read_to_string(project.dir.join("worktrees.nix")).unwrap();
        assert!(!mounted.contains("kept"), "{mounted}");
    }

    #[test]
    fn done_deletes_the_branch_only_when_its_work_is_accepted() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("demo");
        init_git_repo(&repo);
        let project = project(&temp.path().join("sandbox"), &repo);
        let add = |name: &str| {
            add_worktree(
                &project,
                Some(name),
                false,
                None,
                &VmOptions::default(),
                &mut Vec::new(),
            )
            .unwrap()
            .worktree
        };
        let accepted = add("accepted");
        let pending = add("pending");
        let asked = add("asked");
        for worktree in [&accepted, &pending] {
            std::fs::write(worktree.path.join(&worktree.name), "work\n").unwrap();
            crate::git::tests::commit_all(&worktree.path);
        }
        crate::git::tests::must_run_git(&repo, &["cherry-pick", "accepted"]);
        // Its saved snapshot is accepted too, and done prunes it.
        let source =
            crate::sandbox::sync::source_id(&project.dir.canonicalize().unwrap(), "accepted");
        let tip = crate::git::resolve_commit(&repo, "accepted").unwrap();
        let snapshot = format!(
            "agent-review/accepted/20261024-120000-{source}-{}",
            &tip[..12]
        );
        crate::git::tests::must_run_git(&repo, &["branch", &snapshot, "accepted"]);

        let done = |worktree: &Worktree, keep_branch: bool| {
            let options = VmOptions {
                keep_branch,
                ..VmOptions::default()
            };
            super::done(&project, worktree, &options, &mut Vec::new()).unwrap()
        };
        assert_eq!(
            done(&accepted, false),
            [
                Step::Done("worktree: removed accepted and branch accepted".into()),
                Step::Done(format!("snapshot: deleted {snapshot} (on main)")),
            ]
        );
        assert_eq!(
            done(&pending, false)[1],
            Step::Skipped("branch pending kept: 1 commit not on main".into())
        );
        assert_eq!(
            done(&asked, true)[1],
            Step::Skipped("branch asked kept: --keep-branch".into())
        );
        let branches = branches(&repo);
        assert_eq!(branches, ["asked", "main", "pending"]);
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
            None,
            false,
            None,
            &VmOptions::default(),
            &mut Vec::new(),
        )
        .unwrap();
        let worktree = created.worktree;
        assert_eq!(
            done_blocker(&project, &worktree, None, false, &mut Vec::new()).unwrap(),
            None
        );

        std::fs::write(worktree.path.join("wip.txt"), "wip\n").unwrap();
        let blocker = done_blocker(&project, &worktree, None, false, &mut Vec::new())
            .unwrap()
            .unwrap();
        assert!(blocker.contains("?? wip.txt"), "{blocker}");
        std::fs::remove_file(worktree.path.join("wip.txt")).unwrap();

        // Added after the VM started: the guest never had it to pull from.
        std::fs::create_dir_all(&project.state_dir).unwrap();
        std::fs::write(project.state_dir.join("mounted-worktrees.nix"), "{\n}\n").unwrap();
        assert_eq!(
            done_blocker(
                &project,
                &worktree,
                Some(&RunnerState::default()),
                false,
                &mut Vec::new()
            )
            .unwrap(),
            None
        );
        std::fs::write(worktree.path.join("wip.txt"), "wip\n").unwrap();

        std::fs::create_dir_all(&project.state_dir).unwrap();
        std::fs::write(project.state_dir.join("home.img"), "home").unwrap();
        let blocker = done_blocker(&project, &worktree, None, false, &mut Vec::new())
            .unwrap()
            .unwrap();
        assert!(blocker.contains("is stopped"), "{blocker}");
        // A live VM that never mounted this worktree has no history to fetch,
        // even when its persistent home already exists.
        let blocker = done_blocker(
            &project,
            &worktree,
            Some(&RunnerState::default()),
            false,
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
        assert!(blocker.contains("uncommitted changes"), "{blocker}");
        assert_eq!(
            done_blocker(&project, &worktree, None, true, &mut Vec::new()).unwrap(),
            None
        );

        let forced = VmOptions {
            force: true,
            ..VmOptions::default()
        };
        done(&project, &worktree, &forced, &mut Vec::new()).unwrap();
        assert!(project.worktrees().unwrap().is_empty());
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
                worktrees_root: "/wt".into(),
            },
            worktrees: vec![Worktree {
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
                worktrees_root: root.join("worktrees"),
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
