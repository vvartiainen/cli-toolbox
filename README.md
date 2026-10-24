# cli-toolbox

Small Rust CLI for a few daily workflow helpers.

Command parsing and help output are handled with [clap](https://github.com/clap-rs/clap). Interactive selection uses [fzf](https://github.com/junegunn/fzf) when it is on PATH, and a built-in [ratatui](https://github.com/ratatui/ratatui) picker otherwise.

## Requirements

- Rust
- `fzf` for interactive selection (optional; a built-in picker is used if it is missing)
- `kitty` for Kitty-specific commands
- `aws` for AWS helpers
- `az` (Azure CLI) for Azure helpers
- `git` for git helpers
- `rulesync` (38.0.0 tested) for project MCP and skill generation with `agents`
- `herdr` (optional) to open agent worktrees as Herdr workspaces
- `nix` and [agent-sandbox](https://github.com/vvartiainen/agent-sandbox-setup) for sandbox projects (`agent project`)
- `just` for development, and optionally to run a project's `worktree-init` recipe

AWS profile, Azure subscription, Kitty session, and SSH host selection run an interactive terminal UI. Those commands need a TTY.

## Commands

| Command | What it does |
| --- | --- |
| `tui` | Interactive launcher for everything below. See [docs/tui.md](docs/tui.md). |
| `aws profile` | Pick an AWS profile, log in to AWS SSO if needed, and print the `AWS_PROFILE` export. |
| `azure login\|logout\|subscription\|status` | Log in to the Azure CLI (`--force` after PIM activation), log out, pick a subscription (`az account set` plus `AZURE_SUBSCRIPTION_ID`/`ARM_*` exports), or show the current session. Alias `az`. |
| `agents [status\|set\|sync\|library]` | Select project MCP servers and skills through Rulesync. See [docs/agent-tools.md](docs/agent-tools.md). |
| `agent worktree create\|list\|cleanup` | Git worktrees for AI agents, with project init and Herdr, optionally starting an agent with a first prompt. See [docs/agents.md](docs/agents.md). |
| `agent project list\|create\|setup\|start\|stop\|restart\|add\|sync\|done` | Sandbox projects in `~/.agent-sandbox`: one agent-sandbox VM per project, mounting its agent worktrees, with bidirectional commit exchange for review, agent credentials from agent-proxy and a backlog. See [docs/agents.md](docs/agents.md#sandbox-projects). |
| `agent vm list\|stop\|prune` | Every running agent-sandbox VM, also those whose project directory is gone. See [docs/agents.md](docs/agents.md#running-vms). |
| `kitty select-session` | Choose a `*.kitty-session` file from your home directory and switch Kitty to it. |
| `ssh connect` | Choose an SSH host from your config and connect to it with Kitty's SSH kitten. |
| `completion` | Print a shell completion script. |

Running `cli-toolbox` without arguments prints help.

Use `aws profile` via `eval` so the export is applied in your current shell:

```sh
eval "$(cli-toolbox aws profile)"
```

Azure works the same way. Set `AZURE_TENANT_ID` in your shell profile to log in to
that tenant by default (`--tenant` overrides it); without it, `az login` uses your
home tenant.

```sh
export AZURE_TENANT_ID=<tenant-id>
eval "$(cli-toolbox azure subscription)"
cli-toolbox azure login --force   # after activating a PIM role
eval "$(cli-toolbox azure logout)"
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
