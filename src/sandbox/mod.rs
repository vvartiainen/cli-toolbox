//! agent-sandbox integration: the central project flakes and the microVM
//! each one runs. Everything goes through the flake's apps
//! (`nix run .#start|stop|git-pull|herdr-add`, run from the project
//! directory, which identifies the VM) and the `runner.json` files the
//! runner keeps in its host-only runtime directory.

use std::ffi::OsString;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, exec};

pub(crate) mod project;

/// Overrides agent-sandbox's runtime state root, like the runner does.
pub(crate) const RUNTIME_ROOT_ENV: &str = "AGENT_SANDBOX_RUNTIME_ROOT";

/// Host loopback SSH ports handed out to project VMs.
pub(crate) const SSH_PORTS: RangeInclusive<u16> = 2300..=2399;

/// The state the runner of a running VM records in `runner.json`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunnerState {
    pub(crate) pid: u32,
    #[serde(default)]
    pub(crate) ssh_port: Option<u16>,
    pub(crate) name: String,
    pub(crate) dir: PathBuf,
    pub(crate) runtime_dir: PathBuf,
    #[serde(default)]
    pub(crate) console_log: Option<PathBuf>,
}

/// Start the VM of the flake in `dir` in the background and wait until it
/// answers. Starting a running VM reports it again.
pub(crate) fn start(dir: &Path) -> Result<RunnerState, Error> {
    let json = exec::stdout_of_in("nix", &["run", ".#start"], Some(dir), &[])?;
    serde_json::from_str(&json)
        .map_err(|err| Error::msg(format!("parse agent-sandbox start output: {err}\n{json}")))
}

/// Stop the VM of the flake in `dir`. Stopping a stopped VM is not an error.
pub(crate) fn stop(dir: &Path) -> Result<(), Error> {
    exec::run_on_stderr_in("nix", &["run", ".#stop"], Some(dir), &[])
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

/// Whether `pid` is an agent-sandbox runner, the same check the runner uses.
fn runner_alive(pid: u32) -> bool {
    exec::combined_output("ps", &["-p", &pid.to_string(), "-o", "command="], None)
        .is_ok_and(|command| command.contains("agent-sandbox-run"))
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
