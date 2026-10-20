# Agent worktrees and sandboxed agents

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

## Sandboxed agents

Each sandboxed agent gets its own agent worktree, and the worktree runs its own
[agent-sandbox](https://github.com/vvartiainen/agent-sandbox-setup) microVM. herdr
runs in the VM, and your herdr adds the VM as a machine named after the worktree, so
the host herdr is the control plane for every agent. The agent's commits come back as
the worktree's branch, which you review and merge yourself.

```sh
cli-toolbox agent sandbox init --template rust --ai   # once per project
cli-toolbox agent start --agent claude                # worktree + VM + herdr workspace
cli-toolbox agent sync                                # guest commits -> worktree branch
cli-toolbox agent done                                # stop VM, remove worktree, keep branch
```

| Command | What it does |
| --- | --- |
| `agent sandbox init` | Creates `sandbox/flake.nix` from an agent-sandbox template (`--template nodejs\|rust\|go`), points it at agent-sandbox (`--from`, or `CLI_TOOLBOX_AGENT_SANDBOX_FLAKE`), names the VM after the project, authorizes your SSH public key, ignores `.agent-sandbox/` and marks the flake for git. `--ai [KIND]` then starts a coding agent with agent-sandbox's `SETUP-AGENT.md` to tailor the toolchain. Commit the flake when it builds. |
| `agent start` | Creates the next agent worktree, starts its VM in the background on a free SSH port (2300–2399), runs `herdr-add` and opens a herdr workspace at `/workspace/app` in the guest with the agent (`--agent KIND` or `CLI_TOOLBOX_AGENT`). The guest home is fresh; `--home-from main` (or a path) starts from an APFS clone of the main checkout's `.agent-sandbox/home.img`, with its logins. Prints the worktree path. |
| `agent up [name]` / `agent stop [name]` | Boot or stop the VM of an existing worktree. |
| `agent sync [name]` | Runs the flake's `git-pull`: the guest's commits are fetched into `sandbox/*` and the worktree branch is fast-forwarded. Prints the branch's commits beyond the main checkout's `HEAD`. |
| `agent list` | Agent worktrees with branch, VM state and SSH port. |
| `agent done [name]` | Syncs, stops the VM, removes the herdr machine and the worktree, and keeps the branch. It refuses when guest commits have diverged from the branch, when the VM is stopped but has a home image (so it cannot sync), or when the worktree has uncommitted changes; `--force` removes anyway. |

The sandbox flake is `sandbox/flake.nix` (or a root `flake.nix` that uses
agent-sandbox) in the worktree, else in the main checkout; `--flake DIR` overrides
it. VM state comes from agent-sandbox's `runner.json` files under
`~/.local/state/agent-sandbox/vms/`.

### First-time setup

1. Make sure the sandbox flake's `agent-sandbox` input is recent enough to have the
   `start`, `stop` and `status` apps (`nix flake update agent-sandbox --flake ./sandbox`).
2. Put your SSH public key in `authorizedKeys` in `sandbox/flake.nix`
   (`agent sandbox init` does this) and load it in your SSH agent, because herdr
   reconnects without prompting.
3. For herdr to reuse each VM's host key, add this once at the top of
   `~/.ssh/config`:

   ```sshconfig
   Include ~/.local/state/agent-sandbox/vms/*/ssh_config
   ```

4. Build the guest once with `nix build ./sandbox`. This is the slow step.
5. Commit `sandbox/flake.nix` and the `.gitignore` change. New worktrees are created
   from `HEAD`, so they need both.

### Design

| Layer | Owns |
| --- | --- |
| agent-sandbox (Nix) | VM lifecycle for the directory it is started in: ports, state, home image, git bridging, herdr in the guest. Exposes the flake apps `start`, `stop`, `status`, `ssh`, `git-pull` and `herdr-add`, `AGENT_SANDBOX_*` env overrides, and `runner.json`. |
| herdr | Agent sessions in the guest, and the host sidebar of machines. |
| cli-toolbox | Orchestration of worktree, VM, herdr, sync and cleanup, from the CLI and the TUI. |

- cli-toolbox never reads Nix internals. It only runs `nix run <flake>#<app>` with
  the worktree as `cwd`, and reads `runner.json`.
- agent-sandbox knows nothing about worktrees or cli-toolbox.
- Every sandbox step still works by hand with `nix run .#…` in a worktree.
- `.agent-sandbox/` must be gitignored, because `git worktree remove` refuses
  untracked files.

### Known gaps

The end-to-end flow (real VM boot, `herdr-add`, `sync`, `done`) has not been
verified yet. These are the parts most likely to need fixes:

- **`herdr machine list --json` shape.** The code assumes entries with `id` (or
  `profile_id`) and `label`. `agent done` relies on it to remove the machine.
- **herdr calls on a machine.** `herdr --machine … workspace create --cwd /workspace/app`
  and `agent start` were written from the API schema and have not run against a
  real machine.
- **vfkit without a terminal.** `start` runs the VM with stdin from `/dev/null`;
  how the console behaves then is unverified.
- **Leftover runtime directory.** `done --force` on a stopped VM leaves its runtime
  directory behind, because the path is only known while the VM runs.

Manual check: run `agent start` twice (expect two VMs on different ports and two
herdr machines), commit something in a guest, then `agent sync`, `agent list` and
`agent done`.
