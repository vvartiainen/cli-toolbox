# cli-toolbox

Small Rust CLI for a few daily workflow helpers.

Command parsing and help output are handled with [clap](https://github.com/clap-rs/clap). Interactive selection uses [fzf](https://github.com/junegunn/fzf) when it is on PATH, and a built-in [ratatui](https://github.com/ratatui/ratatui) picker otherwise.

## Requirements

- Rust
- `fzf` for interactive selection (optional; a built-in picker is used if it is missing)
- `kitty` for Kitty-specific commands
- `aws` for AWS helpers
- `git` for git helpers
- `herdr` (optional) to open agent worktrees as Herdr workspaces
- `nix` and an [agent-sandbox](https://github.com/vvartiainen/agent-sandbox-setup) flake for sandboxed agents (`agent start`)
- `just` for development, and optionally to run a project's `worktree-init` recipe

AWS profile, Kitty session, and SSH host selection run an interactive terminal UI. Those commands need a TTY.

## Commands

### `tui`

Open the interactive tool launcher:

```sh
cli-toolbox tui
```

Select AWS profiles, switch Kitty sessions, connect to SSH hosts, or create, inspect,
enter, and remove agent worktrees. Worktree operations use the directory from which
you launched the toolbox.

The launcher opens in a compact panel below your prompt instead of taking over the
whole screen. Actions are grouped into tabs: **AWS** (`a`), **Agents** (`i`, agent
worktrees) and **Terminal** (`t`, Kitty sessions and SSH). Press a tab's letter, or
`[`/`]` (also Left/Right, Tab, `h`/`l`) to switch tabs. Then press a number to run
that action directly; for example, `i` `1` opens the agent worktrees. Up/Down (or
`j`/`k`, Ctrl-P/Ctrl-N), PageUp/PageDown, Home/End and Enter also work. Press `/` to
search actions across all tabs and `?` (F1 in lists) to see every key of the current
screen. `q` or Esc exits; Ctrl-C exits at any point while the launcher owns the
terminal.

In resource lists (profiles, hosts, sessions, worktrees), type to filter. Backspace
edits the query and Ctrl-U clears it. F5 refreshes the list and Esc goes back.

**Agents › Worktrees** manages the agent worktrees of the current directory:

- Enter opens a worktree's details. There, Enter (or `c`) shows the `cd` command and
  `d` removes the worktree.
- Ctrl-A creates a new worktree, initializes it and opens it in Herdr (see
  [`agent worktree create`](#agent-worktree-create)). Init output streams to the
  terminal while it runs. Ctrl-D removes the selected worktree and closes its Herdr
  workspace.
- Removal asks for confirmation: `y` removes and `n` or Esc cancels. Tab or
  Left/Right with Enter also works.
- Ctrl-V starts a sandboxed agent ([`agent start`](#sandboxed-agents)). In a
  worktree's details, `s` syncs the agent's commits, `u` and `x` boot and stop
  its VM, and `D` runs `agent done`. The agent kind comes from
  `CLI_TOOLBOX_AGENT`.

The launcher always uses ratatui, whether or not `fzf` is installed. It draws on
stderr and requires stdin, stdout, and stderr to be terminals. SSH and AWS SSO
temporarily take over the terminal; the launcher resumes when they finish.

#### Applying exports and directory changes

By default, AWS exports and worktree directory changes appear as bash/zsh commands to
run after exiting, because the launcher cannot change your parent shell's
environment or directory. With `--print-shell`, a successful AWS profile selection,
worktree creation or worktree `cd` exits the launcher and prints that command to
stdout instead, so a shell function can `eval` it:

```sh
# ~/.zshrc or ~/.bashrc
ct() { local cmd; cmd="$(cli-toolbox tui --print-shell)" || return; [ -n "$cmd" ] && eval "$cmd"; }
```

Only pass `tui --print-shell` output to `eval`; plain `tui` never prints commands.
Running `cli-toolbox` without arguments still prints help.

### `aws`

AWS helpers.

#### `aws profile`

Pick an AWS profile, log in to AWS SSO if needed, and print the `AWS_PROFILE` export for your shell.

Use it via `eval` so the exported variable is applied in your current shell:

```sh
eval "$(cli-toolbox aws profile)"
```

### `agent`

AI agent helpers.

#### Sandboxed agents

Each sandboxed agent gets its own agent worktree, and the worktree runs its own
[agent-sandbox](https://github.com/vvartiainen/agent-sandbox-setup) microVM.
herdr runs in the VM, and your herdr adds the VM as a machine named after the
worktree, so the host herdr is the control plane for every agent. The agent's
commits come back as the worktree's branch, which you review and merge
yourself.

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
agent-sandbox) in the worktree, else in the main checkout; `--flake DIR`
overrides it. VM state comes from agent-sandbox's `runner.json` files under
`~/.local/state/agent-sandbox/vms/`. For herdr to reuse each VM's host key, add
this once at the top of `~/.ssh/config`:

```sshconfig
Include ~/.local/state/agent-sandbox/vms/*/ssh_config
```

#### `agent worktree`

Manage git worktrees for AI agents. Running the command without a subcommand prints help.

Worktrees live in one place, grouped by repository, so they are easy to find and
to run in [agent-sandbox](https://github.com/vvartiainen/agent-sandbox-setup):

```text
~/.agent-sandbox/worktrees/<repo>/<repo>-agent-wt
~/.agent-sandbox/worktrees/<repo>/<repo>-agent-wt-2
```

`<repo>` is the name of the main checkout, so the commands work the same from the
repository and from any of its worktrees. Set `CLI_TOOLBOX_WORKTREES_DIR` to use
another directory. Worktrees made by older versions next to the repository are
still listed and cleaned up.

The previous `git agent-worktree …` commands still work as a hidden alias.

##### `agent worktree create`

Create a new worktree and branch, set it up, then print the worktree path.

```sh
cd "$(cli-toolbox agent worktree create)"
```

In `/path/to/cli-toolbox`, the first call creates `~/.agent-sandbox/worktrees/cli-toolbox/cli-toolbox-agent-wt` on branch `cli-toolbox-agent-wt`. Later calls allocate the next free numbered name.

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
`--no-init` or `--no-herdr`. If a step fails, the worktree is kept, the path is
still printed, and the command exits with an error.

##### `agent worktree list`

List agent worktrees for the current directory.

```sh
cli-toolbox agent worktree list
```

##### `agent worktree cleanup`

Remove an agent worktree and its branch. Pass a worktree name or path to remove one of several, omit the name when only one exists, or use `--all` to remove every agent worktree for this directory. The repository's directory under the worktrees root is removed once it is empty.

If a Herdr workspace has the worktree open, it is closed once the removal succeeds. Git refuses to remove a worktree with uncommitted changes, and the workspace then stays open. Use `--no-herdr` to leave Herdr alone.

```sh
cli-toolbox agent worktree cleanup
cli-toolbox agent worktree cleanup cli-toolbox-agent-wt-2
cli-toolbox agent worktree cleanup --all
```

### `kitty`

Kitty helpers.

#### `kitty select-session`

Choose a `*.kitty-session` file from your home directory and switch Kitty to it.

```sh
cli-toolbox kitty select-session
```

### `ssh`

SSH helpers.

#### `ssh connect`

Choose an SSH host from your config and connect to it with Kitty's SSH kitten.

```sh
cli-toolbox ssh connect
```

## Completions

Generate a completion script for your shell and source it:

```sh
# bash
source <(cli-toolbox completion -c bash)

# zsh
source <(cli-toolbox completion -c zsh)

# fish
cli-toolbox completion -c fish | source
```

`cli-toolbox completion` with no arguments prints the command for your current login shell.

If you install `cli-toolbox` from the Nix flake, bash, zsh, and fish completions are installed with the package.

## Common Development Commands

```sh
just
just build
just test
just fmt
```

## Build

```sh
just build
```

## Run

Invoke the built binary directly:

```sh
./bin/cli-toolbox <arguments>
```

## Nix

If you want to consume this repo as a flake package from a `nix-darwin` or Home Manager setup, see [`NIX-INSTRUCTIONS.md`](NIX-INSTRUCTIONS.md).
