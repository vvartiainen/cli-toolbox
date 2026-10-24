# cli-toolbox

This is a small CLI toolbox / helper written with modern Rust.

The purpose is to extend and help with the functionality of some CLIs I use, for example:

- Read .kitty-session files in home directory and select one of those to change to
- Read SSH host configurations and select one to connect to
- Read AWS profiles and login to one if needed and take it into use
- Log in to the Azure CLI and select a subscription to take into use

## Generic guidelines

1. Use modern Rust (edition 2024)
2. Prefer stdlib features to adding new dependencies
3. Well established dependencies can be used, like clap for CLI command and flag parsing

## Development workflow

1. Start by writing simple happy path tests for the feature
2. Write the implementation
3. Keep iterating the implementation until the tests pass
4. Run `just fmt` and fix the issues
5. Commit the changes using short conventional commit message

## Repo structure

- `src/main.rs`: binary entrypoint
- `src/lib.rs`: library root exposing `run()`
- `src/cli/`: clap command adapters for each top-level CLI area
- `src/aws`, `src/azure`, `src/git`, `src/herdr`, `src/kitty`, `src/project`, `src/sandbox`, `src/ssh`: domain logic and integrations; keep non-trivial behavior here rather than in the command modules
- `src/agent`: git and project worktree workflows that combine git, project init and Herdr; shared by the CLI and the TUI
  - `src/agent/vm.rs`: sandbox projects (one agent-sandbox VM per project mounting its worktrees, + herdr machine): create, add, remove, start, stop, restart, sync, done, update, list
  - `src/agent/review.rs`: review snapshots and their lazygit review checkouts
  - `src/agent/prune.rs`: cleanup of accepted review snapshots, their review checkouts and stale fetch refs
- `src/sandbox`: agent-sandbox integration through the project flake's apps and `runner.json`; `project.rs` discovers and creates the central projects in `~/.agent-sandbox/projects` and writes their `worktrees.nix`; `setup.rs` holds the shared proxy and backlog setup
- `src/select.rs`: shared interactive picker (`fzf` when available, ratatui otherwise)
- `src/exec.rs`: PATH lookup and process helpers
- Tests live next to the module they cover
- `README.md`: overview, command list and development commands
- `docs/`: detailed user docs (`tui.md`, `agents.md`, `nix.md`); ideas and open work are in the Backlog.md backlog, not the repo
- `justfile`: common local development commands
- `flake.nix`: Nix packaging for the CLI

## Change placement

1. Add new executable wiring in `src/main.rs` only when the process entrypoint changes
2. Add or update top-level CLI commands in `src/cli/`
3. Put reusable parsing, file access, external tool integration, and business logic in the matching `src/<area>` module
4. Prefer adding tests beside the module you change, starting with happy path coverage

## Backlog

Ideas and open work are tracked in a Backlog.md backlog through the
`backlog-cli-toolbox` MCP server (`backlog` inside the sandbox), not in the repo.

- Read `backlog://workflow/overview` or call `get_backlog_instructions` first
- Search for an existing task before creating one
- Keep the task status and notes current while working

## Testing safely

Run freely:

- `just fmt`
- `just test`
- `just build`

Do not run `just sandbox` or `just sandbox-ssh`, they start or control a VM.
