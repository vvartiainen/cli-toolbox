# Sandboxed agents: status and manual test steps

Status as of 2026-10-20. The design is in [plans/agent-workflow.md](plans/agent-workflow.md).

## Where things stand

Both repositories are implemented, and nothing is committed in either one.

Automated checks pass:

- **agent-sandbox-setup**: all 11 Darwin checks, including the new `lifecycle-runner` test, which uses a fake VM.
- **cli-toolbox**: `cargo fmt`, `clippy -D warnings` and all 131 tests.

**Not yet verified:** a real VM boot, `herdr-add`, `sync` and `done`. The end-to-end run was stopped while Nix was still building the Linux guest.

## What changed

### agent-sandbox-setup

**Runtime overrides.** These settings can be given when starting, without changing the flake:

| Variable | Effect |
| --- | --- |
| `AGENT_SANDBOX_SSH_PORT` | SSH port for this VM |
| `AGENT_SANDBOX_NAME` | SSH alias and herdr label |
| `AGENT_SANDBOX_HOME_SEED` | Copy this home image with `cp -c` instead of starting fresh |
| `AGENT_SANDBOX_START_TIMEOUT` | How long `start` waits for the VM |
| `AGENT_SANDBOX_STOP_TIMEOUT` | How long `stop` waits before killing it |
| `AGENT_SANDBOX_RUNTIME_ROOT` | Where host-side runtime state is kept |

**Runtime state is host-only.** Each VM gets a directory at `~/.local/state/agent-sandbox/vms/<dir>-<hash>/`, holding `runner.json`, `ssh_config`, `known_hosts` and `console.log`.

- It lives outside the workspace on purpose. The guest can write to the workspace, so a forged `runner.json` there could make `stop` kill a host process.
- The new script `nix/runner/runtime.sh` is sourced by the runner and every helper app.

**New apps: `start`, `stop` and `status`.**

- `start` runs the VM detached and waits for the guest's SSH banner.
- `status` prints JSON.
- Plain `nix run .` is unchanged.

**Existing apps find the running VM by themselves.** `ssh`, `ssh-config`, `git-pull` and `herdr-add` look up the running VM's port and name, and pin its host key in that VM's own `known_hosts`.

**Templates.**

- The examples moved to `templates/{nodejs,go,rust}/flake.nix`. The old `examples/*.nix` files are now symlinks to them.
- The templates set `herdr.enable = true`.
- The flake exports them, so `nix flake init -t <agent-sandbox>#rust` works.

**Docs and tests.**

- `SETUP-AGENT.md` holds the instructions a coding agent follows to tailor a project's flake.
- The README has a new section, "Run several VMs: start, stop, status", and `AGENTS.md` is updated.
- `tests/lifecycle-runner.nix` is new.
- The other runner tests set `HOME`.
- Shellcheck now runs with `-x -P SCRIPTDIR`.

New files were added with `git add -N` so Nix can see them.

### cli-toolbox

**`src/sandbox/mod.rs`** talks to agent-sandbox. It contains:

- flake discovery
- `start`, `stop`, `git_pull` and `herdr_add`
- parsing of `runner.json`
- `running_vms` and `vm_for`
- `free_port`, which hands out ports 2300–2399

**`src/sandbox/init.rs`** implements `agent sandbox init`:

- copies a template and points it at agent-sandbox
- sets the VM name and the SSH key
- adds `.agent-sandbox/` to `.gitignore`
- with `--ai`, starts an agent with the setup instructions

**`src/agent/vm.rs`** implements `agent start`, `up`, `stop`, `sync`, `list` and `done`.

**`src/herdr/mod.rs`** adds:

- `remove_machine`
- `open_machine_workspace`, which creates a workspace in the guest and runs `agent start` there

**`src/git/mod.rs`** adds:

- `RemoveOptions { keep_branch, force }`
- `main_checkout`, `is_ignored`, `resolve_commit`, `is_ancestor`, `status_lines` and `log_oneline`

**TUI**, in the worktree manager:

- `^V` starts a sandboxed agent.
- In a worktree's details, `s` syncs, `u` boots the VM, `x` stops it and `D` runs `done`.

**Other changes.**

- `sandbox/flake.nix` is migrated to the current API: `workspaces.app`, an inline Rust module, and herdr enabled.
- README and `AGENTS.md` are updated.

## Manual test steps

1. **Point this repo's sandbox at the new agent-sandbox.** `sandbox/flake.lock` still pins the old upstream, which lacks the new apps. Choose one option:
   - Push agent-sandbox-setup, then run `nix flake update agent-sandbox --flake ./sandbox`.
   - Point the input at your local checkout for now: `git+file:///Users/vvarti/prog/agent-sandbox-setup`.
2. **Add your SSH public key** to `authorizedKeys` in `sandbox/flake.nix`. It is still `""`. Load the key in your SSH agent too, because herdr reconnects without prompting.
3. **Add this line once, at the top of `~/.ssh/config`:**
   ```sshconfig
   Include ~/.local/state/agent-sandbox/vms/*/ssh_config
   ```
4. **Build the guest once:** `nix build ./sandbox`. This is the slow step that runs on the Linux builder.
5. **Commit `sandbox/flake.nix` and the `.gitignore` change.** New worktrees are created from `HEAD`, so they need both.
6. **Run the workflow:**
   ```sh
   cli-toolbox agent start --agent claude   # --no-herdr tests the VM alone
   cli-toolbox agent list
   # commit something in the guest
   cli-toolbox agent sync
   cli-toolbox agent done
   ```
7. **Test a second VM.** Run `agent start` twice. Expect two VMs on different ports and two herdr machines.

## Likely to need fixes

- **`herdr machine list --json` shape.** The code assumes entries with `id` (or `profile_id`) and `label`. This was not confirmed because the list was empty. `agent done` relies on it to remove the machine.
- **herdr calls on a machine.** The `herdr --machine … workspace create --cwd /workspace/app` and `agent start` calls were written from the API schema and have not been run against a real machine.
- **vfkit without a terminal.** `start` runs the VM with stdin from `/dev/null`. How the console behaves then is unverified.
- **Leftover runtime directory.** `done --force` on a stopped VM leaves its runtime directory behind, because the path is only known while the VM runs.

## Before committing

Commit agent-sandbox-setup first, then update `sandbox/flake.lock` here.
