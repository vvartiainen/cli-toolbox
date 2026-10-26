# Interactive launcher (`tui`)

```sh
cli-toolbox tui
```

Select AWS profiles and Azure subscriptions, switch Kitty sessions, connect to SSH hosts, create, inspect,
enter, and remove git worktrees, run sandbox project VMs, or stop any running
sandbox VM. Git worktree operations use
the directory from which you launched the toolbox.

The launcher opens in a compact panel below your prompt instead of taking over the
whole screen. It always uses ratatui, whether or not `fzf` is installed. It draws on
stderr and requires stdin, stdout, and stderr to be terminals. SSH, AWS SSO and Azure
login temporarily take over the terminal; the launcher resumes when they finish.

## Keys

Actions are grouped into tabs: **AWS** (`a`), **Azure** (`z`, subscriptions and account login/logout), **Git** (`g`, git worktrees and agent tools),
**Sandbox** (`s`, sandbox projects and running VMs) and **Terminal** (`t`, Kitty sessions and SSH).

| Key | Action |
| --- | --- |
| `a` / `z` / `g` / `s` / `t` | Switch to a tab |
| `[` / `]`, Left/Right, Tab, `h`/`l` | Previous / next tab |
| `1`–`9` | Run that action directly, e.g. `g` `1` |
| Up/Down, `j`/`k`, Ctrl-P/Ctrl-N | Move |
| PageUp/PageDown, Home/End | Page, first, last |
| `/` | Search actions across all tabs |
| `?` (F1 in lists and forms) | Show every key of the current screen |
| `q`, Esc | Exit (Ctrl-C exits at any point) |

In resource lists (profiles, hosts, sessions, worktrees), type to filter. Backspace
edits the query and Ctrl-U clears it. F5 refreshes the list and Esc goes back.

## Git › Worktrees

Manages the git worktrees of the current directory, the ones in
`~/.worktrees/<repo>/` and any others you made, but not sandbox project worktrees or
review checkouts (see [agents.md](agents.md#git-worktrees)):

- Enter opens a worktree's details. There, Enter (or `c`) shows the `cd` command and
  `d` removes the worktree.
- Ctrl-A asks for the new worktree's name, and with Herdr for its agent and
  first prompt (see [New worktree](#new-worktree)). It then creates the worktree,
  initializes it and opens it in Herdr. Init output streams to the terminal while
  it runs.
- Ctrl-D removes the selected worktree and its branch, and closes its Herdr
  workspace, after a [confirmation](#confirmations).

## Git › Agent tools

Shows the launch directory repository's `ct agents` choices (see
[agent-tools.md](agent-tools.md)): whether generated files are in sync,
whether they are excluded from Git, then each MCP server, skill and target.
A row shows the effective state, then the repository's choice
(`.cli-toolbox/agents.json`, committed) and yours (personal, kept outside the repository):
`on`, `off`, or `–` to inherit. Yours wins over the repository's, which wins
over the catalog default; yours is highlighted when it overrides.

```text
mcp      nixos              [on]   repo on   mine –
mcp      backlog-nix-config [off]  repo on   mine off
```

Ctrl-R and Ctrl-T cycle a layer: from `–` to the opposite of what it
inherits, then the inherited value, then `–` again. Choices are saved right
away; generated files change only when you sync. A target with nothing left
to generate gets no file: sync removes MCP sections it emptied, files left
empty and their empty directories.

| Key | Action |
| --- | --- |
| Ctrl-R | Cycle the repository choice |
| Ctrl-T | Cycle your own choice |
| Enter | On the sync row, sync; on the Git row, toggle excluding generated files |
| Ctrl-S | Sync generated agent files |
| Ctrl-D | Preview the sync without changing files |

## Sandbox › Projects

Lists the sandbox projects in `~/.agent-sandbox/projects/` with their VM state and
worktree count (see [agents.md](agents.md#sandbox-projects)). The Ctrl keys act on
the highlighted project:

- Ctrl-A creates a project for the launch directory's repository (see
  [New project](#new-project)). It has no worktree yet. Next it asks how to tailor
  `flake.nix` to the repository before the first start: `a` runs a coding agent
  with agent-sandbox's setup instructions (`CLI_TOOLBOX_AGENT`, default claude;
  like `--ai`), `e` opens it in `$VISUAL` or `$EDITOR` (default vi), and `s` skips.
  Either way the project's details open next, ready for `a` to add a worktree. The
  VM does not start by itself.
- Ctrl-S syncs the project, Ctrl-R restarts its VM and Ctrl-X stops it. Restart
  and stop ask first while the VM runs.
- Enter opens a project: its repository, VM state and worktrees. Up/Down select a
  worktree. The keys there:

| Key | Action |
| --- | --- |
| `a` | Add a worktree, asking for its name, agent and first prompt ([New worktree](#new-worktree)) |
| `u` | Start the VM and open every worktree in herdr |
| `x` | Stop the VM, after a [confirmation](#confirmations) |
| `v` | Review the selected agent in lazygit in a separate host checkout (or pick a saved snapshot when the VM is stopped or the worktree removed) |
| Enter, `c` | Show the selected worktree's `cd` command |
| `D` | `agent project done`: save its work as snapshots, remove it, delete its branch when accepted |
| `d` | Remove the selected worktree without saving its work; asks whether to keep its branch |
| `m` | Open the command menu below |

Leaving the result of a command run here returns to the project, reloaded and with
the same worktree selected, or the next one when it was removed. Commands run from
a list return to that list with its filter and selection.

The less frequent commands are in the command menu (`m`). Press a command's key,
or move to it and press Enter; Esc goes back to the project. A command, its
confirmation or form returns to the project, not the menu.

| Key | Command |
| --- | --- |
| `s` | Sync: publish the agents' committed work as host review snapshot branches |
| `r` | Restart the VM, after a [confirmation](#confirmations) |
| `t` | Tailor `flake.nix` again with a coding agent (`a`) or the editor (`e`); Esc goes back |
| `f` | Update the flake's inputs (`nix flake update`) |
| `p` | Prune review snapshots whose work is on the integration branch; shows a dry run first |

Worktrees without an agent of their own get `CLI_TOOLBOX_AGENT` when the project
enables it. The project's proxy and backlog setup is not a key; run
`agent project setup` after fixing a failed step.

## Sandbox › VMs

Lists every running agent-sandbox VM with its SSH port, runner PID and start
directory, including VMs whose project directory was removed, which are marked
`(directory gone)` (see [agents.md](agents.md#running-vms)):

- Enter or Ctrl-X stops the selected VM and removes its herdr machine
  (`agent vm stop`), after a [confirmation](#confirmations).
- Ctrl-D removes the runtime directories of stopped VMs whose start directory is
  gone (`agent vm prune`).

## New worktree

Creating a worktree (Ctrl-A in Git › Worktrees, `a` in a sandbox project) opens a
short form:

- **Name**: the branch, such as `feature/login`. The folder is the name with `/`
  replaced by `-` (`feature-login`), shown next to it when they differ. For a git
  worktree it starts empty; for a sandbox project it suggests the next
  `<repo>-agent-wt-N`. An existing branch is checked out, otherwise it is created.
- **Agent** (with Herdr): Left/Right choose the agent, or none. A git worktree
  offers the kinds Herdr supports; a sandbox project only offers the agents its
  `flake.nix` enables (`ai.<kind>.enable = true`). It starts as
  `CLI_TOOLBOX_AGENT` when that is on offer, or none.
- **Prompt** (with Herdr): the agent's first prompt, which is optional.

Tab and Up/Down switch between the name and the prompt. Backspace, Ctrl-W and
Ctrl-U delete a character, a word or everything of the field. `?` is text here; F1
shows the keys. Enter creates the worktree once it has a name, and Esc goes back.

A git worktree starts the agent in its Herdr workspace right away, like
`git worktree create NAME --agent KIND --prompt TEXT`. A sandbox project's worktree
starts it when the VM opens the worktree: press `u` to start a stopped VM or `m` `r`
to restart a running one (see [agents.md](agents.md#agents-per-worktree)).

## New project

Ctrl-A in Sandbox › Projects opens the project form for the launch directory's
repository:

- **Name**: the project and guest host name `agent-<name>`, from the checkout
  folder.
- **Template**: `nodejs`, `rust` or `go`, from the repository's files.
- **Proxy**: agent credentials from agent-proxy on the host.
- **Backlog**: a Backlog.md folder for the agents; on when there is a backlog
  root.

Tab and Up/Down move between the fields; Left/Right or Space change the template or
turn a switch on or off. Enter creates the project, like `agent project create`
with `--name`, `--template`, `--no-proxy` and `--no-backlog`, and Esc goes back.

## Confirmations

Actions that are hard to undo ask first, and name what they affect:

- Stopping or restarting a VM (`x`, `m` `r`, Ctrl-X, Ctrl-R, and Enter in the VM
  list) lists the VM: its name, SSH port, project folder and worktree count.
  Restart is included because it also ends the VM's agent sessions.
- Removing a git worktree shows its path and branch.
- Removing a project worktree with `d` warns that uncommitted changes and unsynced
  guest commits are lost, and that its saved snapshots stay until prune. `k`
  keeps the branch and `d` deletes it.
- Pruning with `m` `p` shows the dry run: what would be deleted and what kept, and
  why. `p` deletes the accepted snapshots and their review checkouts; `s` also the
  superseded ones; `F` force-deletes everything listed, also unaccepted snapshots
  and review checkouts with edits, whose work is then lost.

A choice's key runs it right away. Tab or Left/Right with Enter also picks one;
Cancel is selected first, so Enter alone does nothing. `n` or Esc cancels.
Up/Down scroll a long report.

## Applying exports and directory changes

By default, AWS and Azure exports and worktree directory changes appear as bash/zsh commands to
run after exiting, because the launcher cannot change your parent shell's
environment or directory. With `--print-shell`, a successful AWS profile or Azure subscription selection, Azure logout,
worktree creation or worktree `cd` exits the launcher and prints that command to
stdout instead, so a shell function can `eval` it:

```sh
# ~/.zshrc or ~/.bashrc
ct() { local cmd; cmd="$(cli-toolbox tui --print-shell)" || return; [ -n "$cmd" ] && eval "$cmd"; }
```

Only pass `tui --print-shell` output to `eval`; plain `tui` never prints commands.
