//! agent-sandbox integration: the project's sandbox flake and the microVM it
//! runs for a directory. Everything goes through the flake's apps
//! (`nix run <flake>#start|stop|git-pull|herdr-add`) and the `runner.json`
//! files the runner keeps in its host-only runtime directory.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::net::TcpListener;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, exec};

pub(crate) mod init;

/// Overrides agent-sandbox's runtime state root, like the runner does.
pub(crate) const RUNTIME_ROOT_ENV: &str = "AGENT_SANDBOX_RUNTIME_ROOT";

/// Host loopback SSH ports handed out to agent VMs.
pub(crate) const SSH_PORTS: RangeInclusive<u16> = 2300..=2399;

/// Where a project keeps its sandbox flake, relative to the checkout.
const FLAKE_DIRS: [&str; 2] = ["sandbox", "."];

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

/// Runtime settings for `start`, passed as `AGENT_SANDBOX_*` variables.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StartOptions {
    pub(crate) ssh_port: Option<u16>,
    pub(crate) name: Option<String>,
    pub(crate) home_seed: Option<PathBuf>,
}

/// The directory of the sandbox flake in `checkout`: `sandbox/flake.nix`, or
/// a root `flake.nix` that uses agent-sandbox.
pub(crate) fn find_flake(checkout: &Path) -> Option<PathBuf> {
    FLAKE_DIRS.iter().find_map(|dir| {
        let dir = checkout.join(dir);
        let text = fs::read_to_string(dir.join("flake.nix")).ok()?;
        text.contains("agent-sandbox").then(|| normalize(&dir))
    })
}

fn normalize(dir: &Path) -> PathBuf {
    if dir.ends_with(".") {
        dir.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        dir.to_path_buf()
    }
}

/// Start the VM for `dir` in the background and wait until it answers.
pub(crate) fn start(
    dir: &Path,
    flake: &Path,
    options: &StartOptions,
) -> Result<RunnerState, Error> {
    let port = options.ssh_port.map(|port| port.to_string());
    let mut envs: Vec<(&str, &OsStr)> = Vec::new();
    if let Some(port) = &port {
        envs.push(("AGENT_SANDBOX_SSH_PORT", OsStr::new(port)));
    }
    if let Some(name) = &options.name {
        envs.push(("AGENT_SANDBOX_NAME", OsStr::new(name)));
    }
    if let Some(seed) = &options.home_seed {
        envs.push(("AGENT_SANDBOX_HOME_SEED", seed.as_os_str()));
    }
    let json = exec::stdout_of_in("nix", &["run", &app(flake, "start")?], Some(dir), &envs)?;
    serde_json::from_str(&json)
        .map_err(|err| Error::msg(format!("parse agent-sandbox start output: {err}\n{json}")))
}

/// Stop the VM for `dir`. Stopping a stopped VM is not an error.
pub(crate) fn stop(dir: &Path, flake: &Path) -> Result<(), Error> {
    exec::run_on_stderr_in("nix", &["run", &app(flake, "stop")?], Some(dir), &[])
}

/// Fetch the guest's commits into `dir`'s repository as `sandbox/*` and
/// fast-forward its branch. Returns the app's report.
pub(crate) fn git_pull(dir: &Path, flake: &Path) -> Result<String, Error> {
    exec::stdout_of_in("nix", &["run", &app(flake, "git-pull")?], Some(dir), &[])
}

/// Add the VM for `dir` to the host herdr as the machine `label`, or
/// reconnect it if it is already saved.
pub(crate) fn herdr_add(dir: &Path, flake: &Path, label: &str) -> Result<(), Error> {
    exec::run_on_stderr_in(
        "nix",
        &["run", &app(flake, "herdr-add")?, "--", label],
        Some(dir),
        &[],
    )
}

fn app(flake: &Path, name: &str) -> Result<String, Error> {
    let flake = flake
        .to_str()
        .ok_or_else(|| Error::msg(format!("nix needs a UTF-8 path: {}", flake.display())))?;
    Ok(format!("{flake}#{name}"))
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
pub(crate) fn vm_for(root: &Path, dir: &Path) -> Option<RunnerState> {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    running_vms(root).into_iter().find(|state| state.dir == dir)
}

/// Whether `pid` is an agent-sandbox runner, the same check the runner uses.
fn runner_alive(pid: u32) -> bool {
    exec::combined_output("ps", &["-p", &pid.to_string(), "-o", "command="], None)
        .is_ok_and(|command| command.contains("agent-sandbox-run"))
}

/// A free SSH port in `ports`: not used by a running VM and bindable now.
pub(crate) fn free_port(ports: RangeInclusive<u16>, running: &[RunnerState]) -> Option<u16> {
    ports
        .filter(|port| !running.iter().any(|vm| vm.ssh_port == Some(*port)))
        .find(|port| TcpListener::bind(("127.0.0.1", *port)).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNER_JSON: &str = r#"{
        "pid": 4242, "vmPid": 4243, "sshPort": 2301, "name": "demo-agent-wt",
        "dir": "/wt/demo-agent-wt", "runtimeDir": "/state/vms/demo-agent-wt-abc",
        "consoleLog": "/state/vms/demo-agent-wt-abc/console.log",
        "startedAt": "2026-10-20T10:00:00Z", "running": true
    }"#;

    #[test]
    fn parses_runner_json() {
        let state: RunnerState = serde_json::from_str(RUNNER_JSON).unwrap();
        assert_eq!(state.pid, 4242);
        assert_eq!(state.ssh_port, Some(2301));
        assert_eq!(state.name, "demo-agent-wt");
        assert_eq!(state.dir, Path::new("/wt/demo-agent-wt"));
        let without_ssh =
            r#"{"pid": 1, "sshPort": null, "name": "n", "dir": "/d", "runtimeDir": "/r"}"#;
        let state: RunnerState = serde_json::from_str(without_ssh).unwrap();
        assert_eq!(state.ssh_port, None);
        assert_eq!(state.console_log, None);
    }

    #[test]
    fn finds_the_sandbox_flake_before_a_root_flake() {
        let checkout = tempfile::tempdir().unwrap();
        assert_eq!(find_flake(checkout.path()), None);

        let uses_sandbox = "inputs.agent-sandbox.url = \"github:x/agent-sandbox\";";
        fs::write(checkout.path().join("flake.nix"), "{ outputs = _: { }; }").unwrap();
        assert_eq!(find_flake(checkout.path()), None);

        fs::write(checkout.path().join("flake.nix"), uses_sandbox).unwrap();
        assert_eq!(find_flake(checkout.path()), Some(checkout.path().into()));

        fs::create_dir(checkout.path().join("sandbox")).unwrap();
        fs::write(checkout.path().join("sandbox/flake.nix"), uses_sandbox).unwrap();
        assert_eq!(
            find_flake(checkout.path()),
            Some(checkout.path().join("sandbox"))
        );
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
    fn free_port_skips_running_vms_and_bound_ports() {
        let held = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy = held.local_addr().unwrap().port();
        let mut vm: RunnerState = serde_json::from_str(RUNNER_JSON).unwrap();
        vm.ssh_port = Some(busy + 1);

        // The bound port and the running VM's port are both skipped.
        let found = free_port(busy..=busy + 20, &[vm]).unwrap();
        assert!(found > busy + 1, "{found}");
        assert_eq!(free_port(busy..=busy, &[]), None);
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
        assert_eq!(vm_for(root.path(), Path::new("/wt/demo-agent-wt")), None);
    }
}
