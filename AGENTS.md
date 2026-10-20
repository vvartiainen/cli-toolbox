# cli-toolbox

This is a small CLI toolbox / helper written with modern Rust.

The purpose is to extend and help with the functionality of some CLIs I use, for example:

- Read .kitty-session files in home directory and select one of those to change to
- Read SSH host configurations and select one to connect to
- Read AWS profiles and login to one if needed and take it into use

## Generic guidelines

1. Use modern Rust (edition 2024)
2. Prefer stdlib features to adding new dependencies
3. Well established dependencies can be used, like clap for CLI command and flag parsing

## Development workflow

1. Start by writing simple happy path tests for the feature
2. Write the implementation
3. Keep iterating the implementation until the tests pass
4. Run `just fmt` and fix the issues

## Repo structure

- `src/main.rs`: binary entrypoint
- `src/lib.rs`: library root exposing `run()`
- `src/cli/`: clap command adapters for each top-level CLI area
- `src/aws`, `src/git`, `src/herdr`, `src/kitty`, `src/project`, `src/sandbox`, `src/ssh`: domain logic and integrations; keep non-trivial behavior here rather than in the command modules
- `src/agent`: agent worktree workflows that combine git, project init and Herdr; shared by the CLI and the TUI
  - `src/agent/vm.rs`: sandboxed agents (worktree + agent-sandbox VM + herdr machine): start, up, stop, sync, done, list
- `src/sandbox`: agent-sandbox integration through the sandbox flake's apps and `runner.json`; `init.rs` creates a project's sandbox flake
- `src/select.rs`: shared interactive picker (`fzf` when available, ratatui otherwise)
- `src/exec.rs`: PATH lookup and process helpers
- Tests live next to the module they cover
- `README.md`: overview, command list and development commands
- `docs/`: detailed user docs (`tui.md`, `agents.md`, `nix.md`) and `todo.md`
- `justfile`: common local development commands
- `flake.nix`: Nix packaging for the CLI

## Change placement

1. Add new executable wiring in `src/main.rs` only when the process entrypoint changes
2. Add or update top-level CLI commands in `src/cli/`
3. Put reusable parsing, file access, external tool integration, and business logic in the matching `src/<area>` module
4. Prefer adding tests beside the module you change, starting with happy path coverage
