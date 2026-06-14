# cli-toolbox

This is a small CLI toolbox / helper written with modern Golang.

The purpose is to extend and help with the functionality of some CLIs I use, for example:

- Read .kitty-session files in home directory and select one of those to change to
- Read SSH host configurations and select one to connect to
- Read AWS profiles and login to one if needed and take it into use

## Generic guidelines

1. Use modern Golang features  (>=1.26.0)
2. Prefer stdlib features to adding new dependencies
3. Well established dependencies can be used, like Kong for CLI command and flag parsing

## Development workflow

1. Start by writing simple happy path tests for the feature
2. Write the implementation
3. Keep iterating the implementation until the tests pass
4. Run go formatting and linting commands and fix the issues

## Repo structure

- `cmd/cli-toolbox/main.go`: binary entrypoint
- `internal/cli/run.go`: root CLI wiring and execution flow
- `internal/cli/awscmd`, `internal/cli/kittycmd`, `internal/cli/sshcmd`: command-facing adapters for each top-level CLI area
- `internal/aws`, `internal/kitty`, `internal/ssh`: domain logic and integrations; keep non-trivial behavior here rather than in the command packages
- `*_test.go` files live next to the package they cover
- `README.md`: end-user usage and development commands
- `NIX-INSTRUCTIONS.md`: consuming the package from Nix flakes / `nix-darwin`
- `justfile`: common local development commands
- `flake.nix`: Nix packaging for the CLI

## Change placement

1. Add new executable wiring in `cmd/cli-toolbox` only when the process entrypoint changes
2. Add or update top-level CLI commands in `internal/cli/...`
3. Put reusable parsing, file access, external tool integration, and business logic in the matching `internal/<area>` package
4. Prefer adding tests beside the package you change, starting with happy path coverage
