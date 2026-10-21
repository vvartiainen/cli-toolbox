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
  state/<project>/   # home.img (/home/agent), logs
  worktrees/<repo>/<repo>-agent-wt[-N]
```

Set `CLI_TOOLBOX_SANDBOX_DIR` to use another directory than `~/.agent-sandbox`.
The VM is always started from the project directory, which is how agent-sandbox
identifies it.

```sh
cli-toolbox agent project create --ai             # in the repository, once
cli-toolbox agent project start --agent claude    # VM + herdr workspace per worktree
cli-toolbox agent project add --restart           # another worktree for another agent
cli-toolbox agent project sync                    # guest commits -> worktree branches
cli-toolbox agent project done cli-toolbox-agent-wt-2
```

| Command | What it does |
| --- | --- |
| `agent project list` | Projects with VM state and SSH port, and their worktrees. |
| `agent project create` | Creates `projects/<name>/flake.nix` with `nix flake init -t <agent-sandbox>#<template>` (`--template nodejs\|rust\|go`, default from the repository's files), points it at agent-sandbox (`--from`, or `CLI_TOOLBOX_AGENT_SANDBOX_FLAKE`), keeps state in `state/<name>`, picks an SSH port no other project uses (2300–2399), authorizes your SSH public key and mounts `worktrees.nix`; template `guestPaths` such as `app/target` become per-worktree. Then creates the first agent worktree. `--ai [KIND]` starts a coding agent with agent-sandbox's `docs/setup-agent.md` to tailor the toolchain. Prints the project directory. |
| `agent project start [project]` | Writes `worktrees.nix`, runs the flake's `start` app (detached; waits for guest SSH), runs `herdr-add` and opens a herdr workspace at `/workspace/<worktree>` for every worktree, starting the agent (`--agent KIND` or `CLI_TOOLBOX_AGENT`) in each. |
| `agent project stop\|restart [project]` | Stops or restarts the VM. A restart mounts worktrees added or removed since the start. |
| `agent project add [project]` | Creates the next agent worktree of the project's repository and prints its path. A running VM mounts it after a restart; `--restart` does that right away (it also restarts the other agents' sessions). |
| `agent project sync [project]` | Runs the flake's `git-pull`: the guest's commits are fetched into `sandbox/*` and each worktree branch is fast-forwarded (`--worktree NAME` for one). Reports branches that diverged. |
| `agent project done [worktree]` | Syncs the worktree, removes it and keeps its branch. It refuses when guest commits have diverged from the branch, when the VM is stopped but has a guest home (so it cannot sync), or when the worktree has uncommitted changes; `--force` removes anyway. The VM stops when its last worktree is done. |

`[project]` defaults to the project of the current repository, then to the only
project. VM state comes from agent-sandbox's `runner.json` files under
`~/.local/state/agent-sandbox/vms/`.

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
  project directory, and reads `runner.json`.
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
  real machine.
- **Template init steps.** With several worktrees, an init step without a
  `directory` runs in `/workspace`, where the templates' steps skip themselves.
  `--ai` asks the agent to write per-worktree steps.
- **Closing guest workspaces.** `done` does not close the worktree's herdr
  workspace in the guest; it goes away with the next restart.
