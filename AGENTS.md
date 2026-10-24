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

## Working inside a sandbox

A shell under `/workspace/<name>` may be inside the microVM, not on the
host. Check `git rev-parse --git-dir`: sandbox Git directories live under
`/home/agent/.local/share/agent-sandbox/git/`. Do not assume host tools,
credentials, builders or the centrally managed project flake are available.
Do not try to reach the host's hidden `.git` or weaken the sandbox to get them.

### Commits and handoff

Workspace files are shared with the host immediately; Git metadata is not.
A guest commit does **not** update the host branch or index.

- Start with `git status --short` and `git branch --show-current`. Preserve
  existing user changes and use the assigned worktree branch; do not switch
  branches or reset shared files without agreement.
- Make small, focused conventional commits, e.g. `fix(ai): ...` or
  `docs: ...`, following `git log`. Commit completed work unless the user
  requests otherwise. Stage explicit paths, review `git diff --cached`, and
  never include credentials, runtime state or unrelated changes.
- Use the configured Git identity. If committing is blocked, report the
  problem rather than inventing an identity or claiming a commit exists.
- Report the workspace, branch, commit IDs, checks run and checks blocked.
  Keep the VM running until the host has fetched the commits.
- The **user on the host** fetches commits for review from the centrally
  managed project flake with `nix run .#git-fetch -- <workspace>` (or omit
  the workspace for all). Add `--watch` for periodic fetches. Run it from
  that flake's directory, not this repository by assumption. This is a
  host-state operation: agents must not run it from the guest.
  It imports `sandbox-review/<source-id>/*` refs without moving host branches
  or the index. Fetching is not acceptance: integration is the user's choice.
  The legacy `git-pull` app can advance host branches, so do not use it as
  the automatic sync command.
- Do not push to a remote or merge/rebase diverged host history unless asked.
  A new guest branch is fetched but does not automatically replace the host's
  checked-out branch. Detached HEAD commits are not exported by `git-fetch`;
  put completed work on an agreed branch before handoff.

See [host-owned Git synchronization](docs/plans/host-git-sync.md) for the
approved fetch-only policy, watch lifecycle and review commands.

### Guest validation and authentication

Use the safe recipes below when their tools are available. If `just`, Nix
features, store access or the required builder are unavailable, report the
specific blocker and leave the corresponding `just` commands for the host.
Do not substitute raw Nix builds, install a daemon, enable nested VMs or
change host configuration to bypass these limits. Run available local checks
and review the diff; never describe blocked checks as passing.

Claude's proxy credentials are placeholders, not a guest login. A successful
`claude auth status` alone does not prove the host credential works. Follow
[proxy troubleshooting](docs/proxy.md#claude-authentication-failures); never
print tokens, dump the environment, read secret payloads or commit login files.

## Testing safely

These recipes only evaluate or build this repository. They read no host
secrets and start no VM, so run them freely, roughly cheapest first:

- `just fmt`
- `just show`
- `just eval`: evaluates every flake output, including the Linux checks
- `just check`: Darwin regression checks; run after any change
- `just build`: builds the `./local` runner without starting it
- `just check-guest`: Linux VM checks for guest-side changes (network,
  access, secrets, init, proxy, workspaces). Needs an `aarch64-linux`
  builder; if none is available, say so instead of skipping silently

Do not run these; they touch host secrets or host state, so the user runs
them manually:

- `just run`: resolves host secrets and starts the VM
- `just ssh`, `just herdr-add`: act on a running VM and the host herdr
- `just proxy-setup`, `just secrets-init`: write secrets, tokens and config
- `nix run` of any runner app, which starts or controls a VM

Run checks through `just` rather than raw `nix build` so the command matches
what the user has allowed.
