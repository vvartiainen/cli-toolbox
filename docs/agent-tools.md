# Opt-in MCP servers and skills

`ct agents` turns MCP servers and skills from a central catalog on per
repository, and gives them to coding agents when they start. Nothing is
written to the repository or to the agents' global settings, so other
repositories and other developers are unaffected, and whatever a repository
commits for its agents keeps working alongside. `ct` below is an alias for
`cli-toolbox`. A Git repository is required for choices; subdirectories and
linked worktrees resolve to their own checkout, and each checkout has its own
choices.

```sh
ct agents                          # Toggle choices with fzf or the built-in picker
ct tui                             # Git › Agent tools: the same, in the launcher
ct agents status                   # What this repository uses
ct agents set mcp nixos enable
ct agents set skills review disable
ct agents launch claude -- <args>  # Start an agent with them
ct agents library install          # Download remote skills, keeping locked revisions
ct agents library update           # Download remote skills, refreshing revisions
```

## Starting agents

`ct agents launch <agent> [--bin <path>] [-- <args>]` replaces itself with the
agent, its options for the chosen tools first, then `<args>`. `--bin` names the
executable, which matters when a wrapper of the same name comes first on
`PATH`. Outside a repository the agent starts unchanged; if the choices cannot
be read, it starts without them after a warning.

| Agent | MCP servers | Skills |
| --- | --- | --- |
| `claude` | `--plugin-dir`: a `ct-agents` plugin with both | Same plugin, named `ct-agents:<skill>` |
| `codex` | `-c mcp_servers.<name>=…` | Not at launch |
| `copilot` | `--additional-mcp-config` | `--plugin-dir`: a `ct-agents` plugin |
| `pi` | Not at launch | `--skill <path>` |
| `opencode` | Not at launch | Not at launch |

What an agent cannot load is skipped with a note on stderr. OpenCode sessions
share a background server that keeps the configuration it started with, so
tools given at launch would leak between repositories. Pi's management
commands (`pi mcp`, `pi install` and the like) start without options.

Plugins live in the checkout's private directory, named after a hash of their
content, so launches with the same choices share one.

## Files and catalog

| File | Purpose |
| --- | --- |
| `$XDG_CONFIG_HOME/cli-toolbox/agents/catalog.json` | Central catalog |
| `$XDG_DATA_HOME/cli-toolbox/agents/library/` | Installed skills, locks and receipt |
| `$XDG_STATE_HOME/cli-toolbox/agents/projects/<name>-<hash>/choices.json` | Choices of one checkout |
| `$XDG_STATE_HOME/cli-toolbox/agents/projects/<name>-<hash>/launch/` | Plugins for its launches |

XDG directories default to `~/.config`, `~/.local/share` and `~/.local/state`.
The private directory is named after the checkout's folder and a hash of its
path; moving or renaming a checkout starts it with no choices.

A minimal catalog:

```json
{
  "version": 1,
  "mcp": {
    "nixos": { "type": "stdio", "command": "/nix/store/...-mcp-nixos/bin/mcp-nixos" }
  },
  "sources": []
}
```

Servers are stdio servers with `command` and optional `args` and `env`; store
paths are fine, since nothing leaves the machine. Choices files look like
`{"version":1,"mcp":{"nixos":"enable"}}`. Choices of names the catalog no
longer has are ignored. MCP and skill names use lowercase letters and digits
separated by single hyphens or underscores; aliases such as `some-server` and
`some_server` cannot coexist.

## Skill library

Local skills need no download. Map names to absolute directories containing
`SKILL.md`, typically Nix store paths:

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
```

Rulesync installs into the library's `.rulesync/skills/.curated/`, outside agent
discovery paths. `rulesync.lock` and, for npm sources, `rulesync-npm.lock.json` retain
Rulesync revisions and integrity metadata. Back these files up to reproduce your
installation elsewhere. `install` starts with existing locks; only `update` passes
Rulesync's `--update`. Each operation builds and validates a replacement library
beside the existing one, then swaps it in. Download or validation failures leave
the previous installation intact. Removing sources and installing clears their
cached skills.

Launches use only the installed copies and never download. Changing sources
requires another library install before choices can be read. The receipt checks
library content, executable bits and locks; local library edits are reported.
Install again to restore locked content.

## Nix integration boundary

nix-config generates the catalog (MCP servers with store paths, skill sources
and `localSkills`), installs Rulesync and starts the agents through
`ct agents launch`. cli-toolbox has no runtime dependency on nix-config.

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
