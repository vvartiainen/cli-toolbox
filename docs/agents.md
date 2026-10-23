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
cli-toolbox agent project sync                    # guest commits -> worktree branches
cli-toolbox agent project done cli-toolbox-agent-wt-2
```

| Command | What it does |
| --- | --- |
| `agent project list` | Projects with VM state and SSH port, and their worktrees. |
| `agent project create` | Creates `projects/<name>/flake.nix` with `nix flake init -t <agent-sandbox>#<template>` (`--template nodejs\|rust\|go`, default from the repository's files), points it at agent-sandbox (`--from`, or `CLI_TOOLBOX_AGENT_SANDBOX_FLAKE`), keeps state in `state/<name>`, picks an SSH port no other project uses (2300–2399), authorizes your SSH public key and mounts `worktrees.nix`; template `guestPaths` such as `app/target` become `*/target` (every worktree) and template init steps get `eachWorkspace = true`, so the guest system names no worktree and adding or removing one rebuilds only the host runner. Adds a cli-toolbox defaults module (see [Shared setup](#shared-setup)). Then creates the first agent worktree (with `--agent`/`--prompt` for its agent; see [Agents per worktree](#agents-per-worktree)) and runs `agent project setup`. `--ai [KIND]` starts a coding agent with agent-sandbox's `docs/setup-agent.md` to tailor the toolchain. Prints the project directory. |
| `agent project setup [project]` | Sets up the host side of the flake's proxy credentials and backlog; see [Shared setup](#shared-setup). Safe to re-run. |
| `agent project start [project]` | Writes `worktrees.nix`, runs the flake's `start` app (detached; waits for guest SSH), runs `herdr-add` and opens a herdr workspace at `/workspace/<worktree>` for every worktree, starting the worktree's own agent (see [Agents per worktree](#agents-per-worktree)), or else `--agent KIND` or `CLI_TOOLBOX_AGENT`, in each. |
| `agent project stop\|restart [project]` | Stops or restarts the VM. Stopping signals the runner like `agent vm stop`, so it works without a worktree to mount (after the last `done`), and removes the VM's machine from the host herdr; the next start adds it again. A restart mounts worktrees added or removed since the start. `--all` stops or restarts the VM of every project whose VM is running, one after another; a project that fails does not stop the others. |
| `agent project add [project]` | Creates the next agent worktree of the project's repository and prints its path. A running VM mounts it after a restart; `--restart` does that right away (it also restarts the other agents' sessions). `--agent KIND` and `--prompt TEXT` choose its agent and first prompt; see [Agents per worktree](#agents-per-worktree). |
| `agent project sync [project]` | Runs the flake's `git-pull`: the guest's commits are fetched into `sandbox/*` and each worktree branch is fast-forwarded (`--worktree NAME` for one). Reports branches that diverged. `--all` syncs every project whose VM is running, under a `<project>:` heading each. |
| `agent project done [worktree]` | Syncs the worktree, removes it and keeps its branch. It refuses when guest commits have diverged from the branch, when the VM is stopped but has a guest home (so it cannot sync), or when the worktree has uncommitted changes; `--force` removes anyway. The VM stops when its last worktree is done. |

`[project]` defaults to the project of the current repository, then to the only
project. VM state comes from agent-sandbox's `runner.json` files under
`~/.local/state/agent-sandbox/vms/`.

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
| `agent vm stop [name]` | Stops a VM the way the `stop` app does: SIGTERM to its runner, then SIGKILL to the runner and its VM process after `AGENT_SANDBOX_STOP_TIMEOUT` seconds (default 30). Removes its herdr machine. `name` is the host alias (`agent-<project>`) or the start directory's name; without it, the only running VM or one picked. `--all` stops every VM. |
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
| cli-toolbox | Projects, worktrees and the `worktrees.nix` that ties them together; orchestration of VM, herdr, sync and cleanup, from the CLI and the TUI. |

- cli-toolbox never reads Nix internals. It only runs `nix run .#<app>` in the
  project directory, and reads `runner.json`; `agent vm stop` signals the
  runner it records, as the `stop` app does.
- `vm.workspaces` and `stateDir` are fixed when the flake is built, so mounting a
  new worktree needs a VM restart. cli-toolbox remembers what the running VM
  mounted (`state/<project>/mounted-worktrees.nix`) and says when a restart is due.
- agent-sandbox knows nothing about worktrees or cli-toolbox. Every step still
  works by hand with `nix run .#…` in the project directory.

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
