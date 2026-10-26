# Project MCP and skills

`ct agents` manages project choices in Rust, using [Rulesync](https://github.com/dyoshikawa/rulesync)
for native MCP and skill conversion. `ct` below is an alias for `cli-toolbox`.
Install Rulesync separately and put `rulesync` on `PATH`; version 38.0.0 is tested.
`status` and `set` do not run Rulesync. A Git repository is required for project
commands, including the interactive picker. Subdirectories and linked worktrees
resolve to their own checkout root.

```sh
ct agents                        # Pick choices with fzf or the built-in picker
ct tui                           # Git › Agent tools: toggle, sync, preview
ct agents --shared               # Edit portable choices
ct agents status                 # Effective selections, origins and file drift
ct agents set mcp nixos enable
ct agents set targets pi disable --shared
ct agents set mcp nixos inherit   # Remove the override
ct agents sync --dry-run         # Preview generated file changes
ct agents sync                   # Apply; never downloads skills
ct agents exclude enable         # Keep generated files out of Git here
ct agents library install        # Explicit download, retaining locked revisions
ct agents library update         # Explicit download, refreshing revisions
```

Interactive selection saves only when you select **Save choices**. Escape cancels.
Sync is always explicit. Choices are `inherit`, `enable`, and `disable` and resolve
in this order: central defaults → shared choices → personal choices. A personal
`inherit` leaves the shared choice in effect. Unspecified defaults are disabled.
`--shared` works before or after subcommands and affects only saved choices.

## Files and catalog

| File | Purpose |
| --- | --- |
| `$XDG_CONFIG_HOME/cli-toolbox/agents/catalog.json` | Central catalog and defaults |
| `$XDG_DATA_HOME/cli-toolbox/agents/library/` | Installed skills, locks and receipt |
| `.cli-toolbox/agents.json` | Shared project choices; commit these when appropriate |
| `.cli-toolbox/agents.local.json` | Personal choices for this checkout |
| `.cli-toolbox/agents.state.json` | Ownership records for this checkout |

XDG directories default to `~/.config` and `~/.local/share`. Personal choices and
ownership are added to Git's local `info/exclude`, located through Git so linked
worktrees work correctly. The exclude file is shared by linked worktrees; their
choices and ownership remain independent. Generated output remains visible to Git.

A minimal catalog:

```json
{
  "version": 1,
  "targets": ["codexcli", "claudecode", "opencode", "cursor", "copilotcli", "pi"],
  "mcp": {
    "nixos": { "type": "stdio", "command": "mcp-nixos" }
  },
  "sources": [],
  "defaults": {
    "targets": {
      "codexcli": "enable",
      "claudecode": "enable",
      "opencode": "enable",
      "cursor": "enable",
      "copilotcli": "enable",
      "pi": "enable"
    },
    "mcp": {},
    "skills": {}
  }
}
```

Choices files use `{"version":1,"mcp":{"nixos":"enable"}}` and optionally
`targets` and `skills` maps. Personal choices may also set `"exclude": true`. Unknown names, groups, choices and versions are errors.
MCP and installed skill names use lowercase letters and digits separated by single
hyphens or underscores. Ambiguous MCP aliases such as `some-server` and
`some_server` cannot coexist. Use choices to enable servers; do not put
`enabled`, `disabled`, or `targets` on catalog MCP entries.

## Excluding generated output

Generated MCP files can contain machine-specific paths, such as Nix store
commands. `ct agents exclude enable` records `"exclude": true` in personal
choices; each sync then lists the untracked files it owns in a marked block of
Git's local `info/exclude`. Tracked files are never listed, since excluding
them has no effect. `ct agents exclude disable` and the next sync remove the
block. Shared choices cannot set `exclude`.

## Skill library

Local skills need no download. Map names to absolute directories containing
`SKILL.md`, typically Nix store paths:

```json
"localSkills": { "commit-style": "/nix/store/...-commit-style" }
```

They are selected like installed skills and must not share a name with one.
Their content is part of the sync signature, so `status` reports edits.

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
ct agents sync --dry-run
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

Normal sync uses only the installed copies and never calls install or update.
Changing sources requires another library install before project commands can
resolve skill choices. The receipt checks library content, executable bits and
locks; local library edits are reported. Install again to restore locked content.

## Native output and ownership

| Target | MCP section | Skills |
| --- | --- | --- |
| `codexcli` | `.codex/config.toml`: `mcp_servers` | `.agents/skills` |
| `claudecode` | `.mcp.json`: `mcpServers` | `.claude/skills` |
| `opencode` | `opencode.json(c)`: `mcp` | `.opencode/skills` |
| `cursor` | `.cursor/mcp.json`: `mcpServers` | `.cursor/skills` |
| `copilotcli` | `.github/mcp.json`: `mcpServers` | `.github/skills` |
| `pi` | `.pi/mcp.json`: `mcpServers` | `.pi/skills` |

Rulesync generates only MCP and skills from filtered inputs into temporary output
directories. Broad deletion and global mode are disabled. cli-toolbox then merges
native MCP entries into existing settings, preserving unrelated servers,
authentication, profiles and runtime settings. TOML editing preserves unrelated
formatting and comments. Changed JSONC MCP files are rewritten as JSON, so their
comments are lost. OpenCode uses an existing root `.json` or `.jsonc` file; having
both is an error. MCP overrides in `.opencode/opencode.json(c)` must be consolidated
into the root configuration first.

Existing nonempty MCP sections require explicit adoption for each target:

```sh
ct agents sync --dry-run --adopt-mcp codexcli --adopt-mcp claudecode
ct agents sync --adopt-mcp codexcli --adopt-mcp claudecode
```

Adoption preserves unrelated servers and permits selected catalog entries to
replace same-named entries. Subsequent edits to owned entries abort sync. Save
customizations elsewhere, then restore or remove the conflicting managed entry
before retrying. Newly added unrelated servers are preserved.

Every generated skill file has an ownership hash and executable-bit record.
Disabling a skill or target removes only unchanged owned files. Extra files and
independent skills remain. Edited managed files and unowned collisions abort the
entire sync during preflight. Existing checked-in files in a fresh checkout are
unowned and need manual reconciliation. Preserve the ownership manifest; deleting
it loses the evidence needed for safe removal. Symlinks, path escapes, malformed
configurations and corrupt manifests are rejected. Sync checks for files changing
while preparing its plan and rolls back completed writes on I/O failure.

Dry runs change no project files, library, ownership, or Git exclusions, including
when adoption is requested. Status checks selection and file drift without invoking
Rulesync. Selection controls generation; an agent may still discover another
target's shared skill directory through its own discovery rules.

## Nix integration boundary

nix-config generates the catalog (MCP servers with store paths, skill sources
and `localSkills`) and installs Rulesync. cli-toolbox has no runtime
dependency on nix-config.

## Verification

Run the repository checks:

```sh
just test
just fmt
just build
```

The Rust suite ports the prototype's ownership, inheritance, library, worktree,
JSONC, preflight and rollback tests. Real Rulesync lifecycle tests are opt-in:

```sh
AGENTS_RULESYNC_TEST_BIN="$(command -v rulesync)" \
  cargo test agents::tests::rulesync --lib -- --ignored --nocapture
```

The six-target test covers dry-run, enable, repeat and disable, native MCP merges,
independent skills, supporting scripts, executable bits, and unchanged library
content. It allows only Rulesync's `generate` command and sets nonfunctional HTTP
proxies during generation. This checks that normal sync needs no installer or
network access; it is not an OS-level network sandbox.

The second real Rulesync test installs from a temporary local Git repository,
advances its branch, verifies locked install versus explicit update, and checks
that a failed update preserves the library. These tests were executed with the
npm distribution of Rulesync 38.0.0 on macOS; other platforms and Nix's standalone
Rulesync package were not exercised here. nix-config activation and its global
migration remain outside this repository's verification.
