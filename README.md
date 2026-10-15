# cli-toolbox

Small Rust CLI for a few daily workflow helpers.

Command parsing and help output are handled with [clap](https://github.com/clap-rs/clap). Interactive selection uses [fzf](https://github.com/junegunn/fzf) when it is on PATH, and a built-in [ratatui](https://github.com/ratatui/ratatui) picker otherwise.

## Requirements

- Rust
- `fzf` for interactive selection (optional; a built-in picker is used if it is missing)
- `kitty` for Kitty-specific commands
- `aws` for AWS helpers
- `git` for git helpers
- `just` (for development)

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
- Ctrl-A creates a new worktree and Ctrl-D removes the selected one.
- Removal asks for confirmation: `y` removes and `n` or Esc cancels. Tab or
  Left/Right with Enter also works.

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

#### `agent worktree`

Manage sibling git worktrees for AI agents. Running the command without a subcommand prints help.

Worktrees are created next to the current directory as `<cwd>-agent-wt`, `<cwd>-agent-wt-2`, and so on.

The previous `git agent-worktree …` commands still work as a hidden alias.

##### `agent worktree create`

Create a new worktree and branch, then print the worktree path.

```sh
cd "$(cli-toolbox agent worktree create)"
```

If the current directory is `/path/to/cli-toolbox`, the first call creates `/path/to/cli-toolbox-agent-wt` on branch `cli-toolbox-agent-wt`. Later calls allocate the next free numbered name.

##### `agent worktree list`

List agent worktrees for the current directory.

```sh
cli-toolbox agent worktree list
```

##### `agent worktree cleanup`

Remove an agent worktree and its branch. Pass a worktree name or path to remove one of several, omit the name when only one exists, or use `--all` to remove every agent worktree for this directory.

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
