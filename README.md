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

| Command | What it does |
| --- | --- |
| `tui` | Interactive launcher for everything below. See [docs/tui.md](docs/tui.md). |
| `aws profile` | Pick an AWS profile, log in to AWS SSO if needed, and print the `AWS_PROFILE` export. |
| `agent worktree create\|list\|cleanup` | Git worktrees for AI agents, with project init and Herdr. See [docs/agents.md](docs/agents.md). |
| `agent start\|up\|stop\|sync\|list\|done` | Sandboxed agents, one agent-sandbox VM per worktree. See [docs/agents.md](docs/agents.md#sandboxed-agents). |
| `agent sandbox init` | Create a project's agent-sandbox flake. |
| `kitty select-session` | Choose a `*.kitty-session` file from your home directory and switch Kitty to it. |
| `ssh connect` | Choose an SSH host from your config and connect to it with Kitty's SSH kitten. |
| `completion` | Print a shell completion script. |

Running `cli-toolbox` without arguments prints help.

Use `aws profile` via `eval` so the export is applied in your current shell:

```sh
eval "$(cli-toolbox aws profile)"
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

## Development

```sh
just          # list recipes
just build
just test
just fmt
./bin/cli-toolbox <arguments>   # run the built binary
```

## Documentation

- [docs/tui.md](docs/tui.md): the interactive launcher, its keys and the `--print-shell` hook
- [docs/agents.md](docs/agents.md): agent worktrees and sandboxed agents
- [docs/nix.md](docs/nix.md): consuming the package from a Nix flake / `nix-darwin`
- [docs/todo.md](docs/todo.md): ideas and open work
