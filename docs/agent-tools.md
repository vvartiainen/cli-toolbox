# Project MCP servers and skills

`ct agents` bootstraps and manages the MCP servers and skills of a repository's
coding agents. You pick the agents the project uses (targets), and MCP servers
and skills from a central catalog; sync writes them into each target's own
project files, which are committed with the repository like any other agent
configuration. `ct` below is an alias for `cli-toolbox`. A Git repository is
required; subdirectories and linked worktrees resolve to their own checkout.

```sh
ct agents                          # Toggle choices with fzf or the built-in picker, then sync
ct tui                             # Git › Agent tools: toggle, sync, preview
ct agents status                   # Choices and whether the files match them
ct agents set targets claude enable
ct agents set mcp nixos enable
ct agents set skills review enable
ct agents sync --dry-run           # List the changes
ct agents sync                     # --replace: overwrite differing catalog-named entries
ct agents library install          # Download remote skills, keeping locked revisions
ct agents library update           # Download remote skills, refreshing revisions
```

Nothing is on by default, targets included: choose each agent the project
uses. Choices are saved right away; files change only when you sync.

## Targets and their files

| Target | MCP servers | Skills |
| --- | --- | --- |
| `claude` | `.mcp.json` | `.claude/skills` |
| `codex` | `.codex/config.toml` | `.agents/skills` |
| `copilot` | `.github/mcp.json` | `.agents/skills` or `.claude/skills` |
| `cursor` | `.cursor/mcp.json` | `.agents/skills` or `.claude/skills` |
| `opencode` | `opencode.json` (or an existing `opencode.jsonc`) | `.agents/skills` or `.claude/skills` |
| `pi` | `.pi/mcp.json` | `.agents/skills` |

Each skill is copied once per folder, into as few folders as reach every
target: Claude Code reads only `.claude/skills`, Codex and Pi only
`.agents/skills`, and Copilot, Cursor and OpenCode both. With `claude` and
`codex` or `pi` chosen, both folders are written, and Copilot, Cursor and
OpenCode list each skill twice.

Some agents only read project MCP servers in projects you trust: Codex and Pi
ask the first time they start there. Claude Code asks before starting servers
from `.mcp.json` unless they are approved in its settings.

## State

Only the agents' own files go into the repository; commit them like any other
agent configuration. `ct agents` keeps its own state outside the checkout:

| File | Purpose |
| --- | --- |
| `$XDG_STATE_HOME/cli-toolbox/agents/projects/<repo>-<hash>/choices.json` | The repository's choices, shared by its linked worktrees |
| `$XDG_STATE_HOME/cli-toolbox/agents/projects/<repo>-<hash>/locks/<hash>.json` | What sync wrote in one checkout |

`XDG_STATE_HOME` defaults to `~/.local/state`. The directory is named after the
repository and a hash of its Git directory. Sync updates and removes exactly
what the lock records and never overwrites anything else:

- servers and settings it did not write stay, and an existing server with the
  same name and content is adopted;
- a server or skill file it wrote and someone edited, or a different server or
  file in the way, stops the sync; `--replace` overwrites it instead;
- a section, file or folder left empty is removed.

Without saved state, in a fresh clone, on another machine or after moving the
repository, the choices are worked out from the agent files: catalog-named
servers in a target's MCP file turn on that target and server, and
catalog-named skill folders turn on the skill (and Claude Code, for
`.claude/skills`). Agents that read only `.agents/skills` cannot be told
apart, so choose those again. Catalog-named servers and skills that match what
sync would write are adopted; ones that differ, such as copies from an older
catalog, need `ct agents sync --replace`. `ct agents status` says when the
choices were worked out.

Committed files have to work on every machine, so sync refuses servers whose
command, arguments or environment contain absolute or home paths. Catalog
servers use commands on `PATH`.

## Catalog and library

| File | Purpose |
| --- | --- |
| `$XDG_CONFIG_HOME/cli-toolbox/agents/catalog.json` | Central catalog |
| `$XDG_DATA_HOME/cli-toolbox/agents/library/` | Installed skills, locks and receipt |

XDG directories default to `~/.config` and `~/.local/share`. A minimal catalog:

```json
{
  "version": 1,
  "mcp": {
    "nixos": { "type": "stdio", "command": "mcp-nixos" }
  },
  "sources": []
}
```

Servers are stdio servers with `command` and optional `args` and `env`. MCP
and skill names use lowercase letters and digits separated by single hyphens
or underscores; aliases such as `some-server` and `some_server` cannot
coexist. Choices of names the catalog no longer has are ignored.

Local skills need no download. Map names to absolute directories containing
`SKILL.md`, typically Nix store paths; sync copies their content, so no path
reaches the repository:

```json
"localSkills": { "commit-style": "/nix/store/...-commit-style" }
```

They are chosen like installed skills and must not share a name with one.

For remote skills, add [Rulesync skill sources](https://rulesync.dyoshikawa.com/guide/declarative-sources.html)
to the catalog, then install them explicitly:

```json
"sources": [
  { "source": "anthropics/skills", "skills": ["skill-creator"] }
]
```

```sh
ct agents library install
ct agents set skills skill-creator enable
ct agents sync
```

Rulesync installs into the library's `.rulesync/skills/.curated/`, outside agent
discovery paths. `rulesync.lock` and, for npm sources, `rulesync-npm.lock.json` retain
Rulesync revisions and integrity metadata. Back these files up to reproduce your
installation elsewhere. `install` starts with existing locks; only `update` passes
Rulesync's `--update`. Each operation builds and validates a replacement library
beside the existing one, then swaps it in. Download or validation failures leave
the previous installation intact. Removing sources and installing clears their
cached skills.

Sync uses only the installed copies and never downloads. Changing sources
requires another library install before choices can be read. The receipt checks
library content, executable bits and locks; local library edits are reported.
Install again to restore locked content.

## Nix integration boundary

nix-config generates the catalog (portable server commands it installs on
`PATH`, skill sources and `localSkills`) and installs Rulesync. cli-toolbox has
no runtime dependency on nix-config.

## Verification

Run the repository checks:

```sh
just test
just fmt
just build
```

The real Rulesync library test is opt-in:

```sh
AGENTS_RULESYNC_TEST_BIN="$(command -v rulesync)" \
  cargo test agents::tests::rulesync --lib -- --ignored --nocapture
```

It installs from a temporary local Git repository, advances its branch, and
verifies locked install versus explicit update, and that a failed update
preserves the library. It was run with the npm distribution of Rulesync 38.0.0
on macOS.
