# Plan: agent workflow across agent-sandbox, herdr and cli-toolbox

Status: proposal. Supersedes most of `agent-tasks.md` (clone, bundles, registry).

## Context

All the pieces exist, but they are not connected yet:

- **agent-sandbox-setup** (uncommitted work in progress) has `agentSandbox.workspaces`, which mounts host dirs at `/workspace/<name>` with a guest-owned `.git` (this works for git worktrees too). It also has the `git-pull` app, which fetches guest commits into `refs/remotes/sandbox/*` and fast-forwards, and the `herdr-add` app.
- **cli-toolbox** `agent worktree create|cleanup` already creates and removes worktrees under `~/.agent-sandbox/worktrees`, runs init, and opens or closes host herdr workspaces (`src/agent/mod.rs`, `src/herdr/mod.rs`).
- **herdr** runs in the guest. The host herdr adds the VM as a *machine* and acts only as the control plane.

Running one VM per worktree hits three problems today:

1. The SSH port is fixed at flake evaluation (`ssh.hostPort`), so two worktrees of one project collide.
2. The runner can only run in the foreground, with its console on the terminal.
3. Nothing records a running VM's state, so cleanup cannot find it to stop it.

The older `docs/plans/agent-tasks.md` design is mostly superseded by the upstream workspaces and git-pull work: no clone, no bundles, no registry.

User decisions:

- herdr runs in the guest, and the host only adds the VM as a machine.
- The guest home starts **fresh by default**, with an opt-in APFS clone of another image.
- Sync only brings the agent's branch to the host. Review and merge happen manually, separately.

## Layering (who owns what)

| Layer | Owns | Interface it exposes |
|---|---|---|
| agent-sandbox-setup (Nix) | VM lifecycle for *the directory it is started in*: ports, state, home image, git bridging, herdr-in-guest | flake apps `start`, `stop`, `status`, `ssh`, `git-pull`, `herdr-add`; env overrides; `.agent-sandbox/runner.json`; flake `templates` |
| herdr | Agent sessions in the guest, and the host sidebar of machines | its CLI (`machine add/remove`, `--machine X workspace create`, `agent start`) |
| cli-toolbox | Orchestration: worktree, then VM, then herdr, then sync, then cleanup, in CLI and TUI | `agent …` commands, plus TUI actions that call the same `crate::agent` functions |

Rules:

- cli-toolbox never reads Nix internals. It only runs `nix run <flake>#<app>` with `cwd` set to the worktree, and reads `runner.json`.
- agent-sandbox knows nothing about worktrees or cli-toolbox.
- Every sandbox step still works by hand with `nix run .#…` in a worktree.

## Part A: agent-sandbox-setup changes (upstream, do first)

1. **Runtime overrides** in `nix/runner/run.sh`:
   - `AGENT_SANDBOX_SSH_PORT` overrides `@sshPort@`.
   - `AGENT_SANDBOX_NAME` overrides the host alias and herdr label (default `agent-<name>`).
2. **Runner state** at `$stateDir/runner.json`: `{pid, sshPort, name, consoleLog, startedAt}`.
   - Written after the bridges start. Removed in `cleanup()`.
   - `ssh.sh`, `git-pull.sh`, `herdr-add.sh` and `ssh-config.sh` use the env override first, then `runner.json`, then the eval-time default.
3. **Detached lifecycle apps** in `nix/runner/runner.nix` and `nix/lib.nix`:
   - `start`: runs the runner with `setsid`, stdin from `/dev/null`, output to `$stateDir/console.log`. It waits until guest SSH answers, using the same loop as `herdr-add.sh`, then prints `runner.json`. It fails if a VM is already running for this dir.
   - `stop`: sends SIGTERM to the pid in `runner.json`, waits, and falls back to SIGKILL. The existing trap cleans up the bridges and secrets.
   - `status`: prints `runner.json` plus `running: true|false`, and exits non-zero when the VM is stopped.
   - Plain `nix run .` stays interactive and unchanged.
4. **Home seeding**: if the home image is missing and `AGENT_SANDBOX_HOME_SEED=<img>` is set, `cp -c` (APFS clone) from the seed. Default stays fresh.
5. **Templates**: add flake `templates.{nodejs,rust,go}` from `examples/`, so `nix flake init -t <agent-sandbox>#rust` works.
   - Add `SETUP-AGENT.md`: a prompt or skill that guides an AI agent to tailor the toolchain module, `guestPaths` and `init.steps` for a given repo, and to verify with `nix build`.
6. **Docs**: a README section, "Running several worktrees of one project", covering env overrides, `start`/`stop`/`status`, and `runner.json` as a stable contract.
7. **Tests**: extend `tests/ssh-runner.nix` and `tests/workspaces-runner.nix`:
   - The port override is honoured.
   - `runner.json` is written and then removed.
   - `stop` terminates the VM. The test stubs `microvm-run`, as the runner tests already do.

## Part B: cli-toolbox changes

New module `src/sandbox/mod.rs`, the integration layer for agent-sandbox, in the same style as `src/herdr/mod.rs`:

- `find_flake(checkout) -> Option<PathBuf>`: looks for `sandbox/flake.nix` first, then a root `flake.nix` that mentions `agent-sandbox`. `--flake` overrides it.
- `start(worktree, flake, Overrides{port, name, home_seed})`, `stop(worktree, flake)` and `status(worktree) -> Option<RunnerState>`. `status` parses `runner.json` with serde, like `herdr::ListResponse`.
- `git_pull(worktree, flake)` and `herdr_add(worktree, flake, label)`.
- `free_port()`: scans 2300–2399 and skips ports listed in sibling `*/.agent-sandbox/runner.json` under `git::worktrees_root()`, plus ports where a `TcpListener::bind` probe fails. There is no registry file.
- All of these run through `exec::run_on_stderr_in` and `exec::combined_output`.

Additions to `src/herdr/mod.rs`:

- `remove_machine(label)`.
- `open_remote_workspace(label, cwd, name)` and `start_agent(label, workspace, agent)`, using `herdr --machine <label> workspace create --cwd /workspace/app --label <name>`. Confirm the exact flags with `herdr --skill` during implementation.

Orchestration in `src/agent/mod.rs` reuses `Step`/`Options`/`report`/`check_steps`. Add a `sandbox: bool` and `home_seed: Option<PathBuf>` mode to `Options`.

| Command | Steps |
|---|---|
| `agent start [--home-from main\|PATH] [--agent claude] [--flake P]` | 1. `git::create_agent_worktree_record`. 2. Skip host init; deps install in the guest via `init.steps`. 3. `sandbox::start` with a free port, name = worktree name, and the optional seed (`main` = `<main checkout>/.agent-sandbox/home.img`). 4. `sandbox::herdr_add` with label = worktree name. 5. Open a guest workspace and start the agent. 6. Print the path. |
| `agent up [name]` / `agent stop [name]` | Restart or stop the VM of an existing worktree. herdr reconnect is part of up. |
| `agent sync [name]` | 1. `sandbox::git_pull`. 2. Print the branch and `git log --oneline <main-branch>..<branch>`. The branch now lives in the main repo for manual review and merge. |
| `agent list` | Agent worktrees with VM running/stopped, port and herdr label (from `status`). |
| `agent done [name] [--force]` | 1. Sync. 2. If the guest still has commits the host lacks, or `git-pull` reported divergence, refuse without `--force`. 3. `sandbox::stop`. 4. `herdr::remove_machine`. 5. Remove the worktree but **keep the branch** (see below). |

Other changes:

- `src/git/mod.rs`: `remove_agent_worktree` gains a `keep_branch: bool`. `agent done` keeps the branch, and the existing `agent worktree cleanup` keeps today's `branch -D`.
- Note: `git worktree remove` refuses untracked files, so `.agent-sandbox/` must be gitignored. `sandbox init` ensures this, and `start` warns if it is missing.
- `agent sandbox init [--template nodejs|rust|go] [--ai]` (later phase):
  - Runs `nix flake init -t <agent-sandbox>#<t>` into `sandbox/` and adds `.agent-sandbox/` to `.gitignore`.
  - Fills in `name`, a placeholder for the SSH public key (from `~/.ssh/*.pub` after confirmation), and `herdr.enable`.
  - With `--ai`, starts the agent CLI on the host with `SETUP-AGENT.md` as its prompt.
- `src/cli/agent.rs`: new subcommands, thin as today.
- TUI (`src/tui/app.rs`, `actions.rs`): in the Agents tab, add "New sandboxed agent". The worktree manager gets `s` sync, `x` stop/up and `D` done, each through new `Action` variants that call `crate::agent`.
- `README.md`: a workflow section with the four commands.
- `AGENTS.md` repo structure: add `src/sandbox`.

## Phases

1. **Part A items 1–3, 7** (overrides, `runner.json`, start/stop/status). Check by hand that two worktrees run at once on different ports.
2. **cli-toolbox** `src/sandbox`, `agent start/stop/up/list/sync/done`, `keep_branch`.
3. **herdr guest workspace and agent start** in `agent start`.
4. **TUI actions.**
5. **Part A items 4–6** (home seed, templates, `SETUP-AGENT.md`) and `agent sandbox init`.

## Verification

- Unit tests, written happy path first and kept beside each module:
  - `sandbox::status` parses `runner.json`.
  - `free_port` skips used and bound ports.
  - `find_flake` lookup order.
  - `herdr` command builders and `remove_machine`.
  - `agent::done` refuses when unsynced. This test uses a local "guest" repo, as `git-pull.sh` already supports via `AGENT_SANDBOX_GIT_REMOTE_ROOT`.
  - `remove_agent_worktree(keep_branch=true)` keeps the branch.
- Then `just fmt` and `cargo test`.
- Upstream: `just check` in agent-sandbox-setup.
- Manual end-to-end in cli-toolbox (it has `sandbox/flake.nix`):
  1. `agent start` twice gives two VMs and two herdr machines.
  2. Commit in the guest, then `agent sync`, then `git log` on the host shows the branch.
  3. `agent done` stops the VM, removes the machine and the worktree, and keeps the branch.
