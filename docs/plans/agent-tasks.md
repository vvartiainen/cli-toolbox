# Plan: isolated agent task workflow (`cli-toolbox agent`)

Status: proposal. Validation is intentionally light (happy-path unit tests plus a
manual checklist); VM, Herdr and Moshi behaviour is verified by hand.

## What exists today

- `src/git/mod.rs` + `src/cli/agent.rs`: `agent worktree create|list|cleanup`
  (formerly `git agent-worktree`, kept as a hidden alias)
  creates **linked** sibling worktrees. Linked worktrees share the main repo's
  `.git` and are not suitable for VM isolation; the new workflow lives beside
  them and does not replace them.
- `src/exec.rs`: `look_path`, `require`, `run_merged`, `combined_output`,
  `run_on_stderr`. Reuse them for every `git` / `ssh` / `nix` call.
- `sandbox/flake.nix`: uses `agent-sandbox-setup` `lib.mkFlake` (microvm.nix,
  vfkit, virtiofs). That design has one VM per project:
  - `workspace = "."` is shared with the guest as `/workspace`. The path is
    relative, so it resolves against the runner's current directory.
  - The home volume is `.agent-sandbox/home.img`, mounted at `/home/agent`. Only
    one VM may attach an image at a time.
  - SSH runs over vsock and is bridged by `socat` to `127.0.0.1:<ssh.hostPort>`.
    The port is fixed when Nix evaluates the flake (2222 here).
  - Host secrets are written to `$workspace/.agent-sandbox/host-secrets` and
    removed when the VM exits.
  - tmux is already enabled, and `users.users.agent.linger = true`.
  - The Node module installs dependencies at boot into
    `/workspace/<modulePath>`.
- Config format: the toolbox has no config file yet. Its only serialization
  dependency is `serde`/`serde_json`, so **config and state use JSON**. Don't
  add a TOML crate.
- Herdr 0.9.3 is on the Mac. It has `herdr machine add`, `--remote <ssh-target>`,
  `--session <name>`, and `workspace|tab|agent create/start` over its socket
  API. Confirm exact flags with `herdr --skill` while implementing.

## Design decisions

1. **One VM per task.** That is the only way to export only one task's directory
   to a guest. Agents that share a VM can read everything that VM can read.
   Herdr workspaces are not a security boundary, and the README must say so.
2. **Home image per task, cloned from a per-project template.** On `start`, run
   `cp -c` (an APFS clone, so near-free) from
   `~/.local/state/cli-toolbox/agent/homes/<project>.img` to the task's
   `vm/home.img`. Credentials and caches log in once per project. Tasks never
   share a live image. Add a `agent home init <repo>` helper, or document how to
   boot one task and then copy its image back as the template.
3. **Host layout.** Everything under `vm/` and the state dir stays off the
   virtiofs export:
   ```
   ~/.agent-worktrees/<id>/
     workspace/            <- exported as /workspace (only this)
       repo/               <- independent clone, /workspace/repo
       .handoff/           <- guest writes git bundles here
       .agent-sandbox/     <- transient host-secrets (existing upstream mechanism)
     vm/
       home.img            <- per-task guest home (not exported)
       console.log         <- runner stdout/stderr
       runner.pid
     bootstrap.log         <- host copy of the last bootstrap log
   ~/.local/state/cli-toolbox/agent/
     registry.json
     locks/registry.lock, locks/repo-<hash>.lock, locks/task-<id>.lock
     homes/<project>.img
   ```
4. **Upstream change to `agent-sandbox-setup` (Phase 0).** The task VM needs
   three things set at runtime instead of at eval time: the workspace dir, the
   home image path and the SSH host port. Proposal for `nix/runner/run.sh`:
   - `AGENT_SANDBOX_SSH_PORT` overrides `@sshPort@`.
   - `AGENT_SANDBOX_WORKSPACE` and `AGENT_SANDBOX_HOME_IMAGE`: check how the
     microvm vfkit runner bakes the share `source` and volume `image`.
     - If both stay relative, have `run.sh` `cd` into a runner dir.
     - Otherwise make the module options default to values that `run.sh`
       substitutes.
   - Fallback if that turns out to be awkward: `cli-toolbox` generates a tiny
     per-task wrapper flake that imports the project's sandbox modules and
     overrides `workspace`, `persistentHomeImage` and `ssh.hostPort`. Only the
     runner script differs, so the guest system closure is shared.
   - Also add an opt-in `agentSandbox.session.herdr.enable` (package + config)
     and `mosh`, in the upstream repo.
   - Task VMs should set `node.install.enable = false`. The workspace is now
     `/workspace/repo`, and dependency installs move to the bootstrap config.
     Either keep the `node_modules` bind mount via
     `modulePaths = [ "repo/node_modules" ]`, or let bootstrap install into the
     repo, which is guest-only anyway because the clone is not the host checkout.
5. **The clone is the isolation unit.** Steps:
   - Run `git clone --no-hardlinks <source> workspace/repo`. No `--shared`, no
     `--reference`, no alternates. Hooks and config are not copied by clone.
   - Run `git remote remove origin`. The host path means nothing in the guest,
     and no push target is needed.
   - Run `git switch -c agent/<id> <base-sha>`.
   - `--base` defaults to `HEAD` and is resolved to a full SHA, which is
     recorded.
   - If `git status --porcelain` in the source is non-empty, print the files
     and state that uncommitted changes are **not** included. Never stash or
     discard them.
6. **Handoff via git bundles.** These are produced in the guest, and the host
   never runs git inside the agent-controlled clone. Details in "Collect and
   accept" below.
7. **Locks.** Use stdlib `File::lock()` (stable since Rust 1.89) on files in
   `locks/`. No new dependency.
   - Registry read-modify-write: take `registry.lock`.
   - Long per-task operations (bootstrap, collect, remove): take
     `task-<id>.lock`.
   - `accept`: take `repo-<hash-of-canonical-source-path>.lock`.
8. **IDs.**
   - With `--name`: `<repo-name>-<name>`.
   - Without: `<repo-name>-<4 hex>`, from hashing time, pid and a counter with
     std `DefaultHasher`.
   - Uniqueness is checked under the registry lock. Reject names that are not
     `[a-z0-9-]`.

## CLI surface

Extends the top-level `agent` command, next to `agent worktree`:

```
cli-toolbox agent start <repo> [--name NAME] [--base REF] [--agent CLI] [--session herdr|tmux] [--no-attach]
cli-toolbox agent list
cli-toolbox agent attach [<id>]            # picker via select.rs when omitted
cli-toolbox agent shell <id>               # plain guest shell (ssh)
cli-toolbox agent logs <id>                # bootstrap log (+ --console for VM log)
cli-toolbox agent bootstrap <id> --retry   # explicit rerun
cli-toolbox agent stop <id> / up <id>      # stop VM, keep task / boot VM again
cli-toolbox agent collect <id>
cli-toolbox agent accept <id> [--into BRANCH]
cli-toolbox agent remove <id> [--force]
```

## Config

Project file `<repo>/.cli-toolbox/agent.json`, committed with the project:

```json
{
  "sandbox_flake": "./sandbox",
  "agent": "codex",
  "session": "herdr",
  "bootstrap": {
    "commands": ["npm ci"],
    "script": null,
    "env": { "NODE_ENV": "development" },
    "timeout_secs": 1200
  },
  "tabs": { "shell": true, "test": "npm test -- --watch" }
}
```

User overrides in `~/.config/cli-toolbox/agent.json`:

```json
{
  "defaults": { "agent": "claude", "session": "herdr", "worktrees_dir": "~/.agent-worktrees" },
  "projects": { "/Users/me/prog/foo": { "bootstrap": { "timeout_secs": 3600 } } }
}
```

- Merge order: built-in defaults < project file < user `defaults` < user
  `projects[<canonical path>]`. Merge field by field. Structs use
  `Option<…>` + `serde(default)`.
- `commands` and `script` are mutually exclusive. That is the only validation
  needed.
- `sandbox_flake` defaults to `<repo>/sandbox` if it exists, otherwise `<repo>`.

## Registry (`registry.json`)

```json
{ "tasks": { "<id>": {
  "id": "...", "source": "/abs/canonical/repo", "task_dir": "...",
  "clone": ".../workspace/repo", "base_commit": "<sha>", "base_ref": "main",
  "branch": "agent/<id>", "agent": "codex", "session": "herdr",
  "ssh_port": 2301, "ssh_host": "agent-<id>", "runner_pid": 1234,
  "state": "creating|booting|bootstrapping|bootstrap_failed|ready|stopped|accepting|removing",
  "bootstrap": { "status": "ok", "exit_code": 0, "finished_at": "..." },
  "collections": [ { "n": 1, "tip": "<sha>", "ref": "refs/heads/agent/<id>/review-1" } ],
  "created_at": "..."
}}}
```

- Write it atomically: write a temp file in the same dir, then `rename`.
- Pick SSH ports from 2300–2399, skipping ports in the registry and ports where
  `TcpListener::bind` fails.
- SSH config: maintain `~/.local/state/cli-toolbox/agent/ssh_config` with one
  `Host agent-<id>` block per task (`HostName 127.0.0.1`, `Port`,
  `User agent`, `StrictHostKeyChecking accept-new`,
  `UserKnownHostsFile <state>/known_hosts`). Tell the user to add
  `Include ~/.local/state/cli-toolbox/agent/ssh_config` to `~/.ssh/config`.
  `ssh`, Herdr and Moshi then all use the same alias.

## Lifecycle details

### `start`

1. Canonicalize `<repo>`, load the merged config and resolve the base SHA. Warn
   about uncommitted changes.
2. Under the registry lock: allocate the ID and port, write the entry as
   `creating`.
3. Clone (decision 5), create `workspace/.handoff`, then clone the home template
   if it exists. If not, the runner creates a fresh image.
4. Boot the VM with `nix run <sandbox_flake>`:
   - Pass the runtime overrides from Phase 0.
   - Detach it: `setsid`/`nohup`-style `Command`, stdin `/dev/null`, output to
     `vm/console.log`.
   - Record the pid and set the state to `booting`.
   - Poll `ssh agent-<id> true` until it works or a timeout passes.
5. Bootstrap (next section). On failure: set `bootstrap_failed`, print the log
   tail, and print the `logs`, `shell` and `bootstrap --retry` commands. Keep
   everything.
6. Create the session (Herdr or tmux), set `ready`, and attach unless
   `--no-attach`.

### Bootstrap

- Generate a bash script on the host and pipe it to
  `ssh agent-<id> bash -s`. It runs as `agent` in the guest, after systemd has
  mounted `/workspace` and the home volume. Toolchains come from the NixOS
  closure.

  ```bash
  set -euo pipefail
  log=~/.local/state/agent-bootstrap/<id>.log; mkdir -p "${log%/*}"
  exec > >(tee "$log") 2>&1
  cd /workspace/repo
  export NODE_ENV=development     # from config env
  echo "+ npm ci"; timeout 1200 bash -euo pipefail -c 'npm ci'
  # or: timeout 1200 bash scripts/agent-bootstrap.sh
  ```

- Wrap the whole command list in a single `timeout`, so the budget is total,
  not per command.
- Preserve the exit code. The host maps the `ssh` status, so 124 means timed
  out.
- After it finishes, copy the log back to `<task>/bootstrap.log` with
  `ssh … cat`.
- Mark `bootstrap.status = ok` only on exit 0.
- `attach` never reruns bootstrap. `bootstrap <id> --retry` is the only rerun
  path, and it works from both `ready` and `bootstrap_failed`.
- README: bootstrap scripts must be idempotent.
- Caches live under `/home/agent`, which is the persistent per-task image cloned
  from the project template. That covers npm and cargo caches. Linux
  `node_modules` / `target` stay inside the clone, so Darwin builds never see
  them.

### Interactive sessions

- **Herdr (default).** Herdr runs inside the guest.
  - Create the workspace over SSH from the host:
    `ssh agent-<id> herdr --session <id> …` or `herdr --machine agent-<id> workspace create …`.
  - Name the workspace `<id>`, with cwd `/workspace/repo`.
  - Tab `agent` runs the agent CLI. Use `herdr agent start` for supported CLIs,
    otherwise a plain pane command.
  - Optional tabs `shell` and `test` come from config.
  - `attach` = `herdr --remote agent-<id> --session <id>`.
  - Optionally run `herdr machine add agent-<id>` on `start` so the Herdr
    desktop sidebar lists the task. Remove it on `remove`.
- **tmux.**

  ```
  tmux new-session -d -s <id> -n agent -c /workspace/repo '<agent>'
  tmux new-window -t <id> -n shell -c /workspace/repo
  ```

  `attach` = `ssh -t agent-<id> tmux new-session -A -s <id>`. Never start tmux
  inside a Herdr pane, and never start Herdr inside tmux. The session type is
  chosen once per task and stored.
- **Persistence.**
  - Disconnecting (closing a terminal, losing the network) does not stop the
    session. The Herdr server or tmux server keeps running in the guest, and
    linger keeps user services alive.
  - Stopping the VM (`agent stop`, a Mac reboot, `Ctrl-a x`) kills the session.
    `agent up <id>` boots it again and recreates the workspace and tabs. It does
    not rerun bootstrap. The agent's own history and config survive in
    `/home/agent`.
- **Moshi / iOS.**
  - The guest's SSH listens only on Mac loopback (the vsock bridge), and mosh
    needs UDP, which the vsock bridge does not carry.
  - Supported path: make the Mac reachable from the phone (Tailscale or LAN). In
    Moshi, connect to the **Mac**, then run `cli-toolbox agent attach <id>`
    (Herdr remote attach or `ssh -t … tmux attach`). Moshi then holds the
    roaming connection to the Mac, and the guest session is the same one the
    desktop sees.
  - Document the optional guest-side `moshi-hook` (agent notifications). It is
    installed via the sandbox modules, and it reaches the phone only if the
    guest has outbound internet.
  - Direct phone→guest SSH would need a forwarded port bound beyond loopback.
    Leave it out by default and document it as opt-in.

## Collect and accept

### `collect <id>`

1. Take the task lock and run over SSH, in the guest:
   ```bash
   cd /workspace/repo
   git status --porcelain            # reported separately, never collected
   git rev-parse HEAD refs/heads/agent/<id>
   git bundle create /workspace/.handoff/collect-<n>.bundle <base>..agent/<id>
   ```
   - The exclusion base is `<last collected tip>` once one exists. Keep
     `<base>` for simplicity, since the bundle is small anyway.
   - If the range is empty, report "no new commits" and stop.
2. On the host:
   - Pick the bundle path from the registry: `<task_dir>/workspace/.handoff/collect-<n>.bundle`.
   - Destination ref: `refs/heads/agent/<id>/review-<n>`. Never take a path
     from guest output.
   - Run:

     ```
     git -C <source> -c core.hooksPath=/dev/null bundle verify <bundle>
     git -C <source> -c core.hooksPath=/dev/null fetch <bundle> agent/<id>:refs/heads/agent/<id>/review-<n>
     git -C <source> merge-base --is-ancestor <base> agent/<id>/review-<n>
     ```

   - This fetch only creates a ref. The active checkout and index are untouched.
     Host git reads only the bundle file, never the clone's config or hooks.
3. Append to `collections`. Print the review branch, `git log --oneline base..review`,
   and any uncommitted guest files. The interactive session keeps running, and
   collect can run again any number of times.

### `accept <id> [--into BRANCH]`

- Take the per-repo lock, so accepts into one repo are serialized.
- Use the latest review ref. `--into` defaults to `base_ref`.
- Require the source checkout to be on `--into` and clean. If not, refuse and
  print the exact `git switch` / `git merge` commands. Don't touch someone's
  WIP.
- Run `git merge --no-ff -m "agent: <id>" agent/<id>/review-<n>` in the source,
  with normal user git (hooks enabled, since this is the user's own repo).
- On conflict, leave the merge in progress for normal resolution, set state to
  `accepting`, and print hints. Running `accept` again after the user commits
  detects the merge and marks it accepted.

### `remove <id> [--force]`

1. Take the task lock.
2. If the VM is running, use SSH to ask the guest for `git status --porcelain`
   and `git rev-parse agent/<id>`.
   - If the VM is down, read the clone from the host with
     `git -c core.hooksPath=/dev/null -c core.fsmonitor=false --git-dir … rev-parse`
     (read-only, low risk), or boot it.
3. Refuse when there are dirty files, or when the branch tip is not equal to, or
   an ancestor of, the last collected tip. List exactly what would be lost.
   `--force` overrides.
4. Stop the VM: send SIGTERM to `runner_pid`. The runner's trap kills vfkit and
   socat and deletes the secrets file. Wait, then SIGKILL if needed.
5. Delete the task dir. Remove the SSH config block and the Herdr machine.
   Remove the registry entry.
6. Keep review branches in the source; print them so the user can delete them.

## Code placement (files to change)

New:
- `src/cli/agent.rs` (exists; holds `agent worktree`): add the task
  subcommands, which only parse args and call into `crate::agent`.
- `src/agent/mod.rs`: lifecycle orchestration (start, attach, collect and the
  rest).
- `src/agent/config.rs`: JSON config structs and merging.
- `src/agent/registry.rs`: registry model, atomic save, locks, ID and port
  allocation.
- `src/agent/workspace.rs`: clone, base resolution, dirty detection, task dir
  layout.
- `src/agent/vm.rs`: runner launch/stop, SSH readiness, SSH config file.
- `src/agent/bootstrap.rs`: script generation, run, log copy.
- `src/agent/session.rs`: Herdr and tmux command builders.
- `src/agent/handoff.rs`: bundle create (guest script), verify, fetch, accept,
  removal safety checks.
- `docs/plans/agent-tasks.md` (this file).

Changed:
- `src/cli/mod.rs`: `Agent(AgentCmd)` exists; add help/completion tests for the
  task subcommands.
- `src/lib.rs`: `mod agent;`.
- `src/git/mod.rs`: maybe expose small helpers (`git_output`-style) as
  `pub(crate)` so the agent module can reuse them. Otherwise leave it as is.
- `src/tui/*`: optional later. Add an "Agent tasks" entry to the Agents tab of the launcher
  (list/attach/collect). Not in the first cut.
- `README.md`: `agent` command docs, config reference, the SSH `Include`, the
  Moshi path, the security note on shared VMs, idempotent bootstrap.
- `sandbox/flake.nix`: enable Herdr and mosh once upstream supports them, and
  add a `.cli-toolbox/agent.json` for this repo (`cargo fetch` bootstrap).
- `.gitignore`: nothing new. Task state lives outside the repo.
- Upstream `agent-sandbox-setup`: runtime overrides (Phase 0), a Herdr/mosh
  option, documentation for `node.install` with `/workspace/repo`.

## Phases

0. **Upstream runner overrides.** Workspace dir, home image and SSH port at
   runtime, plus the Herdr/mosh packages. Check by hand: two VMs from the same
   flake on different ports with different workspaces.
1. **Registry, config, workspace.** `start --no-boot` (clone only), `list`,
   `remove`. Tests: config merge, registry round-trip, unique IDs, clone records
   the base SHA, has no alternates (`objects/info/alternates` absent) and no
   origin, dirty-source warning.
2. **VM and bootstrap.** Boot, wait for SSH, bootstrap, `logs`, `shell`,
   `bootstrap --retry`, `stop`/`up`. Test: generated script text (ordering,
   `set -euo pipefail`, timeout, env) as a pure function.
3. **Sessions.** Herdr and tmux creation and `attach`. Test: command builders
   only.
4. **Collect, accept, remove safeguards.** Tests use local repos and run the
   "guest" script locally against the clone: collect creates a review ref
   without changing the source's HEAD or index; a second collect creates
   `review-2`; accept merges; remove refuses dirty or uncollected changes and
   `--force` works.
5. **Docs and manual verification.**

## Manual verification checklist

- [ ] Two tasks on different repos run at the same time. Each guest sees only its
      own `/workspace` (`ls /workspace`, no host paths).
- [ ] Two tasks on the same repo get distinct IDs, ports, branches and VMs.
- [ ] Bootstrap runs after mounts, before the agent starts. Its log is readable
      on the host.
- [ ] A failing bootstrap command leaves the task as `bootstrap_failed` and
      keeps the workspace; `logs` and `shell` work; `--retry` succeeds after the
      fix.
- [ ] `attach` twice: no reinstall (the log timestamp is unchanged).
- [ ] Closing the terminal, then attaching again (desktop and Moshi via the
      Mac), returns to the same session.
- [ ] After `stop` / `up`, the session is recreated and bootstrap is not rerun.
- [ ] Collect, then more agent commits, then collect again: `review-1` and
      `review-2`, and the source checkout is untouched.
- [ ] Uncommitted guest files are reported and not collected.
- [ ] A conflicting accept leaves a normal merge conflict in the source repo.
- [ ] `remove` refuses on dirty or uncollected work; `--force` removes the VM,
      task dir, SSH block and registry entry.
- [ ] The guest has no access to the host main repo, `~/.ssh`, `~/.aws` or the
      registry.

## Open questions

- Exact Herdr API flags for workspace/tab/agent creation (check `herdr --skill`).
- Whether microvm's vfkit runner bakes absolute or relative share and volume
  paths. This decides the Phase 0 approach.
- The source requirements were cut off at "6. … Confirm the guest". Fill in any
  remaining verification items.
