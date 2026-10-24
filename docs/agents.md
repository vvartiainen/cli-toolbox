# Agent worktrees and sandboxed agents

For the `agents` command that manages project MCP servers and skills, see
[Project MCP and skills](agent-tools.md).

## `agent worktree`

Manage git worktrees for AI agents. Running the command without a subcommand prints
help. The older `git agent-worktree …` commands still work as a hidden alias.

Worktrees live in one place, grouped by repository, so they are easy to find and to
run in [agent-sandbox](https://github.com/vvartiainen/agent-sandbox-setup):

```text
~/.agent-sandbox/worktrees/<repo>/<repo>-agent-wt
~/.agent-sandbox/worktrees/<repo>/<repo>-agent-wt-2
```

`<repo>` is the name of the main checkout, so the commands work the same from the
repository and from any of its worktrees. Set `CLI_TOOLBOX_WORKTREES_DIR` to use
another directory. Worktrees made by older versions next to the repository are still
listed and cleaned up.

### `agent worktree create`

Create a new worktree and branch, set it up, then print the worktree path.

```sh
cd "$(cli-toolbox agent worktree create)"
```

In `/path/to/cli-toolbox`, the first call creates
`~/.agent-sandbox/worktrees/cli-toolbox/cli-toolbox-agent-wt` on branch
`cli-toolbox-agent-wt`. Later calls allocate the next free numbered name.

The new worktree is then made ready to work in:

1. **Init.** If the project's justfile has a `worktree-init` recipe, it runs in the
   new worktree. `CLI_TOOLBOX_WORKTREE_SOURCE` holds the checkout the worktree was
   created from, so the recipe can copy untracked files such as `.env`:

   ```just
   worktree-init:
       cp "$CLI_TOOLBOX_WORKTREE_SOURCE/.env" .env
       npm ci
   ```

   Without the recipe, the package manager whose lockfile is in the project root
   installs dependencies: `pnpm install --frozen-lockfile`, `yarn install`,
   `bun install`, `npm ci`, or `npm install` for a bare `package.json`.
2. **Herdr.** If `herdr` is installed, the worktree opens as a focused Herdr
   workspace (`herdr worktree open`), grouped with the source repository's workspace.

Progress goes to stderr; only the path goes to stdout. Skip the steps with
`--no-init` or `--no-herdr`. If a step fails, the worktree is kept, the path is still
printed, and the command exits with an error.

To put an agent to work right away, pass `--agent KIND` and, if you like, its first
prompt with `--prompt TEXT`. The agent then starts in the new Herdr workspace and gets
the prompt:

```sh
cli-toolbox agent worktree create --agent codex --prompt "Fix the flaky login test"
cli-toolbox agent worktree create --agent claude --prompt - < task.md
```

`KIND` is any kind `herdr agent start` supports, such as `claude`, `codex` or `pi`.
If you pass `--prompt` without `--agent`, `CLI_TOOLBOX_AGENT` runs it. `--prompt -`
reads the prompt from stdin. Short one-line prompts go to Claude Code and Codex as
their initial prompt argument, so they still run after a startup question such as
folder trust. Other prompts are sent with `herdr agent prompt` once the agent is ready.

### `agent worktree list`

List agent worktrees for the current directory.

```sh
cli-toolbox agent worktree list
```

### `agent worktree cleanup`

Remove an agent worktree and its branch. Pass a worktree name or path to remove one
of several, omit the name when only one exists, or use `--all` to remove every agent
worktree for this directory. The repository's directory under the worktrees root is
removed once it is empty.

If a Herdr workspace has the worktree open, it is closed once the removal succeeds.
Git refuses to remove a worktree with uncommitted changes, and the workspace then
stays open. Use `--no-herdr` to leave Herdr alone.

```sh
cli-toolbox agent worktree cleanup
cli-toolbox agent worktree cleanup cli-toolbox-agent-wt-2
cli-toolbox agent worktree cleanup --all
```

## Sandbox projects

A sandbox project runs one
[agent-sandbox](https://github.com/vvartiainen/agent-sandbox-setup) microVM for a
repository, and that VM mounts every agent worktree of the repository at
`/workspace/<worktree>`. herdr runs in the VM, and your herdr adds it as the machine
`agent-<project>`, with a guest workspace per worktree, so the host herdr is the
control plane for every agent. The agents' commits come back as the worktrees'
branches, which you review and merge yourself.

Projects live centrally, out of the agents' reach, where cli-toolbox discovers them:

```text
~/.agent-sandbox/
  projects/<project>/
    flake.nix        # from an agent-sandbox template, tailored to the repository
    worktrees.nix    # written by cli-toolbox: worktree name -> path
    project.json     # the repository's main checkout
    agents.json      # written by cli-toolbox: worktree name -> agent and first prompt
  state/<project>/   # home.img (/home/agent), logs
  worktrees/<repo>/<repo>-agent-wt[-N]
```

Set `CLI_TOOLBOX_SANDBOX_DIR` to use another directory than `~/.agent-sandbox`.
The VM is always started from the project directory, which is how agent-sandbox
identifies it.

```sh
cli-toolbox agent project create --ai             # in the repository, once
cli-toolbox agent project start --agent claude    # VM + herdr workspace per worktree
cli-toolbox agent project add --restart --agent codex --prompt "Add a --json flag"
cli-toolbox agent project review                  # fetch agent work and open lazygit
cli-toolbox agent project done cli-toolbox-agent-wt-2
```

| Command | What it does |
| --- | --- |
| `agent project list` | Projects with VM state and SSH port, and their worktrees. |
| `agent project create` | Creates `projects/<name>/flake.nix` with `nix flake init -t <agent-sandbox>#<template>` (`--template nodejs\|rust\|go`, default from the repository's files), points it at agent-sandbox (`--from`, or `CLI_TOOLBOX_AGENT_SANDBOX_FLAKE`), keeps state in `state/<name>`, picks an SSH port no other project uses (2300–2399), authorizes your SSH public key and mounts `worktrees.nix`; template `guestPaths` such as `app/target` become `*/target` (every worktree) and template init steps get `eachWorkspace = true`, so the guest system names no worktree and adding or removing one rebuilds only the host runner. Adds a cli-toolbox defaults module (see [Shared setup](#shared-setup)). Then creates the first agent worktree (with `--agent`/`--prompt` for its agent; see [Agents per worktree](#agents-per-worktree)) and runs `agent project setup`. `--ai [KIND]` starts a coding agent with agent-sandbox's `docs/setup-agent.md` to tailor the toolchain. Prints the project directory. |
| `agent project setup [project]` | Sets up the host side of the flake's proxy credentials and backlog; see [Shared setup](#shared-setup). Safe to re-run. |
| `agent project start [project]` | Writes `worktrees.nix`, runs the flake's `start` app (detached; waits for guest SSH), runs `herdr-add` and opens a herdr workspace at `/workspace/<worktree>` for every worktree, starting the worktree's own agent (see [Agents per worktree](#agents-per-worktree)), or else `--agent KIND` or `CLI_TOOLBOX_AGENT`, in each. |
| `agent project stop\|restart [project]` | Stops or restarts the VM. Stopping signals the runner like `agent vm stop`, so it works without a worktree to mount (after the last `done`), and removes the VM's machine from the host herdr. Before shutdown it stops that machine's guest Herdr session and deletes it (for the guest's `default` session, which Herdr cannot delete, it removes its saved `session.json`), discarding the old terminal layout so the next start opens fresh workspaces instead of offering to resume obsolete sessions. Worktrees, guest home files and agent conversation history stay. If session cleanup fails (for example, SSH is unavailable), the failure is reported and shutdown still proceeds; the machine profile is kept so cleanup can be retried while the VM is running. A restart mounts worktrees added or removed since the start. `--all` stops or restarts the VM of every project whose VM is running, one after another; a project that fails does not stop the others. |
| `agent project add [project]` | Creates the next agent worktree of the project's repository and prints its path. A running VM mounts it after a restart; `--restart` does that right away (it also restarts the other agents' sessions). `--agent KIND` and `--prompt TEXT` choose its agent and first prompt; see [Agents per worktree](#agents-per-worktree). |
| `agent project sync [project]` | Publishes committed agent work as ordinary host snapshot branches under `agent-review/<worktree>/…`, ready to open in Git tools or a separate review worktree. Unchanged tips reuse snapshots; existing branches and agent files stay untouched. Normal host branches are sent to guest review refs separately. `--worktree NAME` selects one; `--all` syncs every running project. See [History exchange](#history-exchange). |
| `agent project review [project]` | Fetches the selected agent's snapshot, prepares a separate host review worktree, and opens lazygit directly in its commits view. No manual Git commands or checkouts. `--worktree NAME` selects an agent; otherwise a picker handles multiple choices. When the VM/worktree is unavailable, offers saved snapshots. Requires host `lazygit` and a terminal. See [Review in lazygit](#review-in-lazygit). |
| `agent project done [worktree]` | Preserves committed guest work as host snapshot branches, checks the guest's own index, then removes the agent worktree. Review branches remain; integration into the stale host counterpart is not required. Dirty/detached guest work, failed snapshots, or an unavailable persistent VM block cleanup. `--force` bypasses these safeguards. The VM stops when its last worktree is done. |

`[project]` defaults to the project of the current repository, then to the only
project. VM state comes from agent-sandbox's `runner.json` files under
`~/.local/state/agent-sandbox/vms/`.

### Review in lazygit

When the agent says it has finished, use **Review**, not just Sync:

```sh
cli-toolbox agent project review
# Or select one agent explicitly:
cli-toolbox agent project review --worktree cli-toolbox-agent-wt
```

In the TUI, open the project, select the agent worktree, and press **v**.
No Git commands or branch checkout are needed. Review:

1. Fetches the agent's committed tip and publishes its normal host snapshot branch.
2. Creates or reuses a **separate detached host review worktree** under the
   project's state directory, in `reviews/`.
3. Runs `lazygit log` there, opening its **commits/log view** at the snapshot tip.
4. Returns to the toolbox when you quit lazygit.

Select a commit in lazygit to inspect its changes. The Files/working-tree panel
will normally be clean: the agent's changes are already **committed**. This is
why Review starts in the commits view rather than leaving you at an empty list
of unstaged files. The named snapshot branch is also available in lazygit's
branch list.

Your main checkout and the agent's live worktree are never checked out, reset,
or cleaned by Review. Existing review copies and edits are kept. Reopening the
same snapshot reuses its matching review checkout; a new tip or a repurposed
review directory gets a separate copy. Accepting/merging work remains explicit;
opening Review does not accept it automatically.

Review requires `lazygit` installed **on the host** and an interactive terminal.
It checks that before fetching or creating a checkout. Closing or failing to
launch lazygit does not delete the snapshot or its review files.

With a stopped VM, or after `done` removed the agent worktree, Review offers
saved snapshots for this project instead and labels them as saved, not newly
fetched work. It never silently falls back to an old snapshot after a live fetch
fails. Multiple agents or saved snapshots use a picker; Escape cancels.

Review fetches agent history for viewing. The separate **Sync** action remains
available for publishing snapshots and sending accepted host branch history back
to the agent's review refs, as described below.

### History exchange

**The agent owns its worktree. You review snapshots on the host.** Do not manually
update or checkout the host counterpart of the agent branch: its Git index can
remain stale even though workspace files are shared live.

For snapshot-only publication and bidirectional history exchange, run this **on
the host**, with the VM running (or press `s` in the TUI):

```sh
cli-toolbox agent project sync
# Or: cli-toolbox agent project sync --worktree cli-toolbox-agent-wt
```

Sync prominently prints an ordinary host branch, for example:

```text
Review branch: agent-review/cli-toolbox-agent-wt/20261024-143000-6aefb5b8dd2a-ab12cd34ef56
```

These appear in your normal Git GUI/branch picker. From your own host checkout,
not the agent's worktree, you can also use:

```sh
git branch --list 'agent-review/*'
git show <printed-review-branch>
git diff main...<printed-review-branch>       # compare with your integration branch
```

To browse or build the complete files without disturbing the agent, use the
separate detached worktree command printed by sync, for example:

```sh
git worktree add --detach ../cli-toolbox-review <printed-review-branch>
```

Choose an unused directory for a new review copy. An existing review copy stays
on its original snapshot while the agent continues working. Accept work from
your own host checkout with an explicit merge or selected cherry-picks, just as
with any other branch; sync itself never merges or rebases.

#### Snapshot behavior

- Each changed guest tip gets a new `agent-review/<worktree>/…` branch. Repeated
  syncs of the same tip reuse the previous branch. Rewritten guest history does
  not overwrite earlier snapshots, even if a review branch is checked out.
- Names include the guest commit's date, a project/workspace source identifier,
  and the tip's short object ID. Collisions get a numeric suffix; sync never
  moves or replaces an existing branch.
- The primary guest branch is shown first. Independent extra guest branches
  also get snapshots; duplicate tips and history already contained in the primary
  snapshot do not create extra branches.
- Snapshot branches are local to the host repository, not pushed upstream.
  They survive `done` and are retained until you explicitly delete them.
  No branches or tags are automatically pruned.
- Existing host/guest branches, indexes and working files remain unchanged.
  Snapshots contain **committed** work only; ask the agent to commit before sync
  when you need its latest file edits.
- Patch counts distinguish new guest patches from equivalents already accepted
  on the host. They are hints relative to the host counterpart, not a requirement
  to maintain or integrate that branch. Divergence is a successful sync.

#### Host changes back to the agent

Normal host branches, including `main` when present, are made available in the VM
as `refs/remotes/host-review/<source>/<branch>`. This works even when the host's
agent branch stays stale. The reserved `agent-review/*` snapshot branches, tags
and submodule remotes are not exported, so review snapshots do not loop back.

Ask the agent to inspect and integrate accepted host changes between tasks,
for example `host-review/<source>/main`. That integration remains under the
agent's control; sync never silently rewrites its active checkout.

The underlying guest fetch refs are `refs/remotes/sandbox-review/<source>/*`;
you do not need them for normal review. The runner's `runtimeDir/ssh_config`
supplies the connection and host key. Load the authorized key in your SSH agent:
SSH must work noninteractively. No VM rebuild or external flake update is needed.

Transport/authentication failures still fail sync; other worktrees are attempted.
Snapshots are created before sending host history, so they remain available even
if that export fails. Unmounted worktrees get a restart hint. Detached guest HEAD
is rejected rather than silently omitting unexported work.

#### Finishing an agent worktree

Finish or pause the agent, then run `agent project done`. It saves guest history
into snapshot branches and checks for staged, unstaged and untracked changes
against the **guest's index**. A stale/dirty-looking host counterpart does not
block cleanup: you do not need to merge or cherry-pick into it first.

Snapshots remain available for review and acceptance after the worktree is
removed. Failed snapshots, dirty/detached guest work, or a stopped persistent VM
block normal cleanup. Use `--force` only when deliberately discarding those
safeguards.

#### Trying a new CLI build

On the host, cherry-pick the implementation commit, then run:

```sh
just fmt
just test
just build
./bin/cli-toolbox agent project review --worktree cli-toolbox-agent-wt
# Or publish snapshots without opening lazygit:
./bin/cli-toolbox agent project sync --worktree cli-toolbox-agent-wt
git branch --list 'agent-review/*'
```

Open the printed branch in your Git tools, or use its separate-worktree command.
Repeat sync without new agent commits: it should print the same branch and
create no duplicates. After another agent commit, sync should print a new branch
while the old branch and any review copy remain unchanged.

Local regression tests cover real Git transfers, snapshot reuse, rewritten tips,
branch collisions, separate/reused review checkouts, dirty indexes, extra branches,
cleanup preservation, and export failures. Review launch tests use an injected
viewer; lazygit is not installed in the guest. The live host-to-VM SSH path and
real lazygit UI still need a host test; a sandbox test is not a substitute.

### Agents per worktree

Each worktree can run its own agent. `agent project add` and `agent project create`
take `--agent KIND` and `--prompt TEXT` for the new worktree, just like
[`agent worktree create`](#agent-worktree-create). The worktree is not mounted until
the VM starts or restarts, so cli-toolbox saves the choice in the project's
`agents.json`, next to the flake and out of the agents' reach. When the VM next opens
the worktree in herdr, it starts that agent and sends the prompt:

```sh
cli-toolbox agent project add --agent codex --prompt "Add a --json flag to list"
cli-toolbox agent project restart       # codex starts in the new worktree with the prompt
```

The prompt is sent only once. Later restarts start the same agent without it, and
worktrees without an agent of their own get `--agent` or `CLI_TOOLBOX_AGENT`. A
worktree's entry is dropped once the worktree is gone, so a later worktree that
reuses the name starts clean.

### Shared setup

Every new project gets the same agent setup, from a module `project create` adds
at the top of the flake's `environment`:

- Claude Code and Codex (and Pi, on by default) take their credentials from
  [agent-proxy](https://github.com/vvartiainen/agent-sandbox-setup/blob/main/docs/proxy.md)
  on the host, so nothing logs in inside the guest: `proxy` with the client
  named after the project and `CLAUDE_CODE_OAUTH_TOKEN`/`ANTHROPIC_OAUTH_TOKEN`
  placeholders, and `ai.chatgptLoginFromProxy`. `--no-proxy` leaves this out.
- A Backlog.md backlog for the agents (`ai.backlog` and `vm.backlogDir`) in
  `$CLI_TOOLBOX_BACKLOG_ROOT/<project>`, by default the Obsidian `Agents` folder in
  iCloud when it exists. An empty `CLI_TOOLBOX_BACKLOG_ROOT` or `--no-backlog` leaves
  it out.

`agent project setup` then does the host side of what the flake uses, and
`project create` runs it:

- With `caCert = ./agent-proxy-ca.pem` in the flake, it runs agent-sandbox's
  `scripts/agent-proxy-setup.sh` for `claude-code,codex`: it stores missing secrets
  (prompting for them), creates the client and its keychain token, and writes the CA
  next to the flake. agent-proxy must be running, and on `PATH` or in
  `AGENT_PROXY_FLAKE`.
- With `vm.backlogDir`, it creates that folder and runs `backlog init` in it
  unless it already has a backlog.

Run it again after fixing a failed step, or for a project whose flake you added
these settings to by hand.

### Running VMs

`agent vm` works from those `runner.json` files alone, so it also reaches a VM
whose project or flake directory was removed while it ran, where
`nix run .#stop` no longer can.

| Command | What it does |
| --- | --- |
| `agent vm list` | Every running VM: host alias, runner PID, SSH port, start time and start directory, marked `(directory gone)` when it no longer exists. |
| `agent vm stop [name]` | Stops a VM the way the `stop` app does: SIGTERM to its runner, then SIGKILL to the runner and its VM process after `AGENT_SANDBOX_STOP_TIMEOUT` seconds (default 30). Resets its guest Herdr session before shutdown and removes its herdr machine, like project stop. `name` is the host alias (`agent-<project>`) or the start directory's name; without it, the only running VM or one picked. `--all` stops every VM. |
| `agent vm prune` | Removes the runtime directories (logs, SSH config, known hosts) of stopped VMs whose start directory is gone. Directories of existing projects are kept. |

### First-time setup

1. Put your SSH public key in `authorizedKeys` (`project create` does this) and
   load it in your SSH agent, because herdr reconnects without prompting.
2. For herdr to reuse each VM's host key, add this once at the top of
   `~/.ssh/config`:

   ```sshconfig
   Include ~/.local/state/agent-sandbox/vms/*/ssh_config
   ```

3. Build the guest once with `nix build` in the project directory. This is the
   slow step.
4. If you keep `~/.agent-sandbox` in git, ignore `state/` and `worktrees/`.
   `project create` marks new project files for git, because Nix only sees tracked
   files.

### Design

| Layer | Owns |
| --- | --- |
| agent-sandbox (Nix) | VM lifecycle for the directory it is started in: ports, state, home image, git bridging, herdr in the guest. Exposes the flake apps `start`, `stop`, `status`, `ssh`, `git-pull` and `herdr-add`, and `runner.json`. |
| herdr | Agent sessions in the guest, and the host sidebar of machines. |
| cli-toolbox | Projects, worktrees and the `worktrees.nix` that ties them together; orchestration of VM, herdr, host review snapshot branches and bidirectional Git history exchange over the runner's SSH connection, and cleanup, from the CLI and the TUI. |

- cli-toolbox never reads Nix internals. VM setup runs `nix run .#<app>` in the
  project directory and reads `runner.json`; history exchange runs ordinary Git
  over SSH using the recorded runtime directory's `ssh_config`. `agent vm stop`
  signals the recorded runner, as the `stop` app does.
- `vm.workspaces` and `stateDir` are fixed when the flake is built, so mounting a
  new worktree needs a VM restart. cli-toolbox remembers what the running VM
  mounted (`state/<project>/mounted-worktrees.nix`) and says when a restart is due.
- agent-sandbox knows nothing about worktrees or cli-toolbox. Lifecycle steps
  still work by hand with `nix run .#…` in the project directory; history exchange
  uses ordinary Git review refs instead of relying on the legacy `git-pull` app.

### Known gaps

The end-to-end flow (real VM boot, `herdr-add`, `sync`, `done`) has not been
verified yet. These are the parts most likely to need fixes:

- **herdr calls on a machine.** `herdr --machine … workspace create --cwd /workspace/<worktree>`
  and `agent start` were written from the API schema and have not run against a
  real machine. The same goes for the first prompt (`agent start -- <prompt>` or
  `agent prompt`), locally and on a machine.
- **Template init steps.** With several worktrees, an init step without a
  `directory` runs in `/workspace`, where the templates' steps skip themselves.
  `--ai` asks the agent to write per-worktree steps.
- **Closing guest workspaces.** `done` does not close the worktree's herdr
  workspace in the guest; it goes away with the next restart.
