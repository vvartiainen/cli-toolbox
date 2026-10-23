//! agent-sandbox integration: the central project flakes and the microVM
//! each one runs. Everything goes through the flake's apps
//! (`nix run .#start|git-pull|herdr-add`, run from the project directory,
//! which identifies the VM) and the `runner.json` files the runner keeps in
//! its host-only runtime directory; stopping needs only the latter.

use std::ffi::OsString;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Error, exec};

pub(crate) mod project;

/// Overrides agent-sandbox's runtime state root, like the runner does.
pub(crate) const RUNTIME_ROOT_ENV: &str = "AGENT_SANDBOX_RUNTIME_ROOT";

/// Host loopback SSH ports handed out to project VMs.
pub(crate) const SSH_PORTS: RangeInclusive<u16> = 2300..=2399;

/// How long `stop` waits for a runner before killing it, like the runner.
const STOP_TIMEOUT_ENV: &str = "AGENT_SANDBOX_STOP_TIMEOUT";
const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// The state the runner of a running VM records in `runner.json`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunnerState {
    pub(crate) pid: u32,
    #[serde(default)]
    pub(crate) vm_pid: Option<u32>,
    #[serde(default)]
    pub(crate) ssh_port: Option<u16>,
    pub(crate) name: String,
    pub(crate) dir: PathBuf,
    pub(crate) runtime_dir: PathBuf,
    #[serde(default)]
    pub(crate) console_log: Option<PathBuf>,
    #[serde(default)]
    pub(crate) started_at: Option<String>,
}

/// How a VM went down.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Stopped {
    /// The runner shut the VM down on SIGTERM.
    Terminated,
    /// The runner did not exit in time; it and its VM process were killed.
    Killed,
}

/// Start the VM of the flake in `dir` in the background and wait until it
/// answers. Starting a running VM reports it again.
pub(crate) fn start(dir: &Path) -> Result<RunnerState, Error> {
    let json = exec::stdout_of_in("nix", &["run", ".#start"], Some(dir), &[])?;
    serde_json::from_str(&json)
        .map_err(|err| Error::msg(format!("parse agent-sandbox start output: {err}\n{json}")))
}

/// Fetch the guest's commits of `workspaces` (all when empty) into their
/// host repositories as `sandbox/*` and fast-forward their branches.
/// Returns the app's report.
pub(crate) fn git_pull(dir: &Path, workspaces: &[&str]) -> Result<String, Error> {
    let mut args = vec!["run", ".#git-pull"];
    if !workspaces.is_empty() {
        args.push("--");
        args.extend(workspaces);
    }
    exec::stdout_of_in("nix", &args, Some(dir), &[])
}

/// Add the VM of the flake in `dir` to the host herdr as a machine labelled
/// with its host alias, or reconnect it if it is already saved.
pub(crate) fn herdr_add(dir: &Path) -> Result<(), Error> {
    exec::run_on_stderr_in("nix", &["run", ".#herdr-add"], Some(dir), &[])
}

/// The runtime state root: `$AGENT_SANDBOX_RUNTIME_ROOT`, or
/// `${XDG_STATE_HOME:-~/.local/state}/agent-sandbox`.
pub(crate) fn runtime_root() -> Option<PathBuf> {
    resolve_runtime_root(
        std::env::var_os(RUNTIME_ROOT_ENV),
        std::env::var_os("XDG_STATE_HOME"),
        dirs::home_dir(),
    )
}

fn resolve_runtime_root(
    root: Option<OsString>,
    state_home: Option<OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    let set = |value: Option<OsString>| value.filter(|value| !value.is_empty()).map(PathBuf::from);
    set(root).or_else(|| {
        set(state_home)
            .or_else(|| home.map(|home| home.join(".local/state")))
            .map(|state| state.join("agent-sandbox"))
    })
}

/// The runtime directory name of the VM started from `dir`, as the runner
/// derives it: `<basename>-<first 12 hex digits of sha256(dir)>`.
pub(crate) fn runtime_dir_name(dir: &Path) -> String {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let hash = format!("{:x}", Sha256::digest(dir.as_os_str().as_encoded_bytes()));
    format!("{name}-{}", &hash[..12])
}

/// Stop `vm` by signalling its runner, like the flake's `stop` app but
/// without the directory it was started from: SIGTERM, then SIGKILL to the
/// runner and its VM process after `timeout`.
pub(crate) fn stop_vm(vm: &RunnerState, timeout: Duration) -> Result<Stopped, Error> {
    if !runner_alive(vm.pid) {
        return Err(Error::msg(format!(
            "{}: runner {} is not running",
            vm.name, vm.pid
        )));
    }
    // Only a VM process that belongs to this runtime directory is killed,
    // so a reused PID is left alone.
    let vm_pid = vm.vm_pid.filter(|&pid| {
        exec::combined_output("ps", &["-p", &pid.to_string(), "-o", "command="], None)
            .is_ok_and(|command| command.contains(&*vm.runtime_dir.to_string_lossy()))
    });
    signal(vm.pid, libc::SIGTERM)?;
    let deadline = Instant::now() + timeout;
    while process_exists(vm.pid) {
        if Instant::now() >= deadline {
            let _ = signal(vm.pid, libc::SIGKILL);
            if let Some(pid) = vm_pid {
                let _ = signal(pid, libc::SIGKILL);
            }
            let _ = fs::remove_file(vm.runtime_dir.join("runner.json"));
            return Ok(Stopped::Killed);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(Stopped::Terminated)
}

/// `$AGENT_SANDBOX_STOP_TIMEOUT` seconds, or 30.
pub(crate) fn stop_timeout() -> Duration {
    std::env::var(STOP_TIMEOUT_ENV)
        .ok()
        .and_then(|secs| secs.parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_STOP_TIMEOUT)
}

/// Runtime directories under `root` with no running VM whose start
/// directory is gone: neither the directory recorded in `runner.json` nor
/// one of `keep` (the existing projects) owns them.
pub(crate) fn stale_runtime_dirs(root: &Path, keep: &[PathBuf]) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root.join("vms")) else {
        return Vec::new();
    };
    let kept: Vec<String> = keep
        .iter()
        .map(|dir| runtime_dir_name(&dir.canonicalize().unwrap_or_else(|_| dir.clone())))
        .collect();
    let mut stale: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .filter(|path| {
            let state = fs::read_to_string(path.join("runner.json"))
                .ok()
                .and_then(|text| serde_json::from_str::<RunnerState>(&text).ok());
            if state
                .as_ref()
                .is_some_and(|state| runner_alive(state.pid) || state.dir.exists())
            {
                return false;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            !kept.iter().any(|kept| *kept == name)
        })
        .collect();
    stale.sort();
    stale
}

fn signal(pid: u32, signal: libc::c_int) -> Result<(), Error> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| Error::msg(format!("invalid PID {pid}")))?;
    // SAFETY: kill(2) only takes plain integers.
    if unsafe { libc::kill(pid, signal) } == 0 {
        Ok(())
    } else {
        Err(Error::msg(format!(
            "signal {pid}: {}",
            std::io::Error::last_os_error()
        )))
    }
}

fn process_exists(pid: u32) -> bool {
    // kill(0, …) signals the whole process group, so PID 0 never names a VM.
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// The VMs that are running now, from every `runner.json` under `root`.
pub(crate) fn running_vms(root: &Path) -> Vec<RunnerState> {
    let Ok(entries) = fs::read_dir(root.join("vms")) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let text = fs::read_to_string(entry.ok()?.path().join("runner.json")).ok()?;
            serde_json::from_str::<RunnerState>(&text).ok()
        })
        .filter(|state| runner_alive(state.pid))
        .collect()
}

/// The running VM started from `dir`, if any.
pub(crate) fn vm_for(running: &[RunnerState], dir: &Path) -> Option<RunnerState> {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    running.iter().find(|state| state.dir == dir).cloned()
}

/// Whether `pid` is an agent-sandbox runner, the same check the runner uses:
/// the process exists, and its command line names the runner. Where `ps` is
/// not allowed, as in the Nix build sandbox, the process existing is enough.
fn runner_alive(pid: u32) -> bool {
    if !process_exists(pid) {
        return false;
    }
    exec::combined_output("ps", &["-p", &pid.to_string(), "-o", "command="], None)
        .map_or(true, |command| command.contains("agent-sandbox-run"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNER_JSON: &str = r#"{
        "pid": 4242, "vmPid": 4243, "sshPort": 2301, "name": "agent-demo",
        "dir": "/sandbox/projects/demo", "runtimeDir": "/state/vms/demo-abc",
        "consoleLog": "/state/vms/demo-abc/console.log",
        "startedAt": "2026-10-20T10:00:00Z", "running": true
    }"#;

    #[test]
    fn parses_runner_json() {
        let state: RunnerState = serde_json::from_str(RUNNER_JSON).unwrap();
        assert_eq!(state.pid, 4242);
        assert_eq!(state.ssh_port, Some(2301));
        assert_eq!(state.name, "agent-demo");
        assert_eq!(state.dir, Path::new("/sandbox/projects/demo"));
        let without_ssh =
            r#"{"pid": 1, "sshPort": null, "name": "n", "dir": "/d", "runtimeDir": "/r"}"#;
        let state: RunnerState = serde_json::from_str(without_ssh).unwrap();
        assert_eq!(state.ssh_port, None);
        assert_eq!(state.console_log, None);
    }

    #[test]
    fn runtime_root_matches_the_runner() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(
            resolve_runtime_root(None, None, home.clone()),
            Some("/home/me/.local/state/agent-sandbox".into())
        );
        assert_eq!(
            resolve_runtime_root(None, Some("/xdg".into()), home.clone()),
            Some("/xdg/agent-sandbox".into())
        );
        assert_eq!(
            resolve_runtime_root(Some("/custom".into()), Some("/xdg".into()), home),
            Some("/custom".into())
        );
    }

    #[test]
    fn running_vms_ignores_dead_runners() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("vms/demo-abc");
        fs::create_dir_all(&dir).unwrap();
        // PID 0 is never a runner.
        let dead = RUNNER_JSON.replace("4242", "0");
        fs::write(dir.join("runner.json"), dead).unwrap();
        assert!(running_vms(root.path()).is_empty());
    }

    #[test]
    fn runtime_dir_names_match_the_runner() {
        assert_eq!(
            runtime_dir_name(Path::new(
                "/Users/vvarti/.agent-sandbox/projects/hydrocap-app-template"
            )),
            "hydrocap-app-template-d6be8223f126"
        );
    }

    #[test]
    fn stale_runtime_dirs_skip_existing_projects() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("projects/demo");
        fs::create_dir_all(&project).unwrap();
        let project = project.canonicalize().unwrap();
        let kept = root.path().join("vms").join(runtime_dir_name(&project));
        let orphan = root.path().join("vms/gone-0123456789ab");
        let recorded = root.path().join("vms/other-ba9876543210");
        for dir in [&kept, &orphan, &recorded] {
            fs::create_dir_all(dir).unwrap();
        }
        // A dead runner whose start directory still exists is kept.
        let state = RUNNER_JSON
            .replace("4242", "0")
            .replace("/sandbox/projects/demo", &project.to_string_lossy());
        fs::write(recorded.join("runner.json"), state).unwrap();
        assert_eq!(stale_runtime_dirs(root.path(), &[project]), vec![orphan]);
    }

    #[test]
    fn stop_vm_terminates_the_runner() {
        let dir = tempfile::tempdir().unwrap();
        let runner = dir.path().join("agent-sandbox-run");
        fs::write(
            &runner,
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile :; do sleep 0.1; done\n",
        )
        .unwrap();
        let mut child = std::process::Command::new("/bin/sh")
            .arg(&runner)
            .spawn()
            .unwrap();
        let vm = RunnerState {
            pid: child.id(),
            name: "agent-demo".into(),
            dir: dir.path().into(),
            runtime_dir: dir.path().into(),
            ..RunnerState::default()
        };
        // Reap the runner as it exits, so it does not linger as a zombie.
        let waiter = std::thread::spawn(move || child.wait());
        assert_eq!(
            stop_vm(&vm, Duration::from_secs(10)).unwrap(),
            Stopped::Terminated
        );
        assert!(waiter.join().unwrap().is_ok());
    }

    #[test]
    fn vm_for_matches_the_start_directory() {
        let vm: RunnerState = serde_json::from_str(RUNNER_JSON).unwrap();
        let running = [vm.clone()];
        assert_eq!(
            vm_for(&running, Path::new("/sandbox/projects/demo")),
            Some(vm)
        );
        assert_eq!(vm_for(&running, Path::new("/sandbox/projects/other")), None);
    }
}
