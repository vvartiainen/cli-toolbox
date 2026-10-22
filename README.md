# cli-toolbox

Small Rust CLI for a few daily workflow helpers.

Command parsing and help output are handled with [clap](https://github.com/clap-rs/clap). Interactive selection uses [fzf](https://github.com/junegunn/fzf) when it is on PATH, and a built-in [ratatui](https://github.com/ratatui/ratatui) picker otherwise.

## Requirements

- Rust
- `fzf` for interactive selection (optional; a built-in picker is used if it is missing)
- `kitty` for Kitty-specific commands
- `aws` for AWS helpers
- `git` for git helpers
- `rulesync` (38.0.0 tested) for project MCP and skill generation with `agents`
- `herdr` (optional) to open agent worktrees as Herdr workspaces
- `nix` and [agent-sandbox](https://github.com/vvartiainen/agent-sandbox-setup) for sandbox projects (`agent project`)
- `just` for development, and optionally to run a project's `worktree-init` recipe

AWS profile, Kitty session, and SSH host selection run an interactive terminal UI. Those commands need a TTY.

## Commands

| Command | What it does |
| --- | --- |
| `tui` | Interactive launcher for everything below. See [docs/tui.md](docs/tui.md). |
| `aws profile` | Pick an AWS profile, log in to AWS SSO if needed, and print the `AWS_PROFILE` export. |
| `agents [status\|set\|sync\|library]` | Select project MCP servers and skills through Rulesync. See [docs/agent-tools.md](docs/agent-tools.md). |
| `agent worktree create\|list\|cleanup` | Git worktrees for AI agents, with project init and Herdr. See [docs/agents.md](docs/agents.md). |
| `agent project list\|create\|start\|stop\|restart\|add\|sync\|done` | Sandbox projects in `~/.agent-sandbox`: one agent-sandbox VM per project, mounting its agent worktrees. See [docs/agents.md](docs/agents.md#sandbox-projects). |
| `agent vm list\|stop\|prune` | Every running agent-sandbox VM, also those whose project directory is gone. See [docs/agents.md](docs/agents.md#running-vms). |
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
- [docs/agent-tools.md](docs/agent-tools.md): project MCP and skill choices, library, adoption and ownership
- [docs/nix.md](docs/nix.md): consuming the package from a Nix flake / `nix-darwin`
- [docs/todo.md](docs/todo.md): ideas and open work
