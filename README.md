# cli-toolbox

Small Go CLI for a few daily workflow helpers.

Command parsing and help output are handled with [Kong](https://github.com/alecthomas/kong).

## Requirements

- Go
- `fzf` for interactive selection
- `kitty` for Kitty-specific commands
- `aws` for AWS helpers
- `git` for git helpers
- `just` (for development)

## Commands

### `aws`

AWS helpers.

#### `aws profile`

Pick an AWS profile with `fzf`, log in to AWS SSO if needed, and print the `AWS_PROFILE` export for your shell.

Use it via `eval` so the exported variable is applied in your current shell:

```sh
eval "$(cli-toolbox aws profile)"
```

### `git`

Git helpers.

#### `git agent-worktree`

Manage sibling git worktrees for AI agents. Running the command without a subcommand prints help.

Worktrees are created next to the current directory as `<cwd>-agent-wt`, `<cwd>-agent-wt-2`, and so on.

##### `git agent-worktree create`

Create a new worktree and branch, then print the worktree path.

```sh
cd "$(cli-toolbox git agent-worktree create)"
```

If the current directory is `/path/to/cli-toolbox`, the first call creates `/path/to/cli-toolbox-agent-wt` on branch `cli-toolbox-agent-wt`. Later calls allocate the next free numbered name.

##### `git agent-worktree list`

List agent worktrees for the current directory.

```sh
cli-toolbox git agent-worktree list
```

##### `git agent-worktree cleanup`

Remove an agent worktree and its branch. Pass a worktree name or path to remove one of several, omit the name when only one exists, or use `--all` to remove every agent worktree for this directory.

```sh
cli-toolbox git agent-worktree cleanup
cli-toolbox git agent-worktree cleanup cli-toolbox-agent-wt-2
cli-toolbox git agent-worktree cleanup --all
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

If you want to consume this repo as a flake package from a `nix-darwin` or Home Manager setup, see [`NIX-INSTRUCTIONS.md`](./NIX-INSTRUCTIONS.md).
