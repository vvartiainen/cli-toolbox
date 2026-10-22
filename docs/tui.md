# Interactive launcher (`tui`)

```sh
cli-toolbox tui
```

Select AWS profiles, switch Kitty sessions, connect to SSH hosts, create, inspect,
enter, and remove agent worktrees, run sandbox project VMs, or stop any running
sandbox VM. Worktree operations use
the directory from which you launched the toolbox.

The launcher opens in a compact panel below your prompt instead of taking over the
whole screen. It always uses ratatui, whether or not `fzf` is installed. It draws on
stderr and requires stdin, stdout, and stderr to be terminals. SSH and AWS SSO
temporarily take over the terminal; the launcher resumes when they finish.

## Keys

Actions are grouped into tabs: **AWS** (`a`), **Agents** (`i`, agent worktrees),
**Sandbox** (`s`, sandbox projects and running VMs) and **Terminal** (`t`, Kitty sessions and SSH).

| Key | Action |
| --- | --- |
| `a` / `i` / `s` / `t` | Switch to a tab |
| `[` / `]`, Left/Right, Tab, `h`/`l` | Previous / next tab |
| `1`–`9` | Run that action directly, e.g. `i` `1` |
| Up/Down, `j`/`k`, Ctrl-P/Ctrl-N | Move |
| PageUp/PageDown, Home/End | Page, first, last |
| `/` | Search actions across all tabs |
| `?` (F1 in lists) | Show every key of the current screen |
| `q`, Esc | Exit (Ctrl-C exits at any point) |

In resource lists (profiles, hosts, sessions, worktrees), type to filter. Backspace
edits the query and Ctrl-U clears it. F5 refreshes the list and Esc goes back.

## Agents › Worktrees

Manages the agent worktrees of the current directory (see [agents.md](agents.md)):

- Enter opens a worktree's details. There, Enter (or `c`) shows the `cd` command and
  `d` removes the worktree.
- Ctrl-A creates a new worktree, initializes it and opens it in Herdr. Init output
  streams to the terminal while it runs. Ctrl-D removes the selected worktree and
  closes its Herdr workspace.
- Removal asks for confirmation: `y` removes and `n` or Esc cancels. Tab or
  Left/Right with Enter also works.

## Sandbox › Projects

Lists the sandbox projects in `~/.agent-sandbox/projects/` with their VM state and
worktree count (see [agents.md](agents.md#sandbox-projects)):

- Ctrl-A creates a project for the launch directory's repository, with its first
  worktree (`agent project create`). It then asks how to tailor `flake.nix` to the
  repository before the first start: `a` runs a coding agent with agent-sandbox's
  setup instructions (`CLI_TOOLBOX_AGENT`, default claude; like `--ai`), `e` opens
  it in `$VISUAL` or `$EDITOR` (default vi), and `s` skips. Either way the
  project's details open next, ready for `u`. The VM does not start by itself.
- Enter opens a project: its repository, VM state and worktrees. There, `u` starts
  the VM and opens every worktree in herdr, `x` stops it, `r` restarts it to mount
  worktree changes, `s` syncs the agents' commits, and `a` adds a worktree. Up/Down
  select a worktree; Enter (or `c`) shows its `cd` command and `D` runs
  `agent project done` on it. The agent kind comes from `CLI_TOOLBOX_AGENT`.

## Sandbox › VMs

Lists every running agent-sandbox VM with its SSH port, runner PID and start
directory, including VMs whose project directory was removed, which are marked
`(directory gone)` (see [agents.md](agents.md#running-vms)):

- Enter or Ctrl-X stops the selected VM and removes its herdr machine
  (`agent vm stop`).
- Ctrl-D removes the runtime directories of stopped VMs whose start directory is
  gone (`agent vm prune`).

## Applying exports and directory changes

By default, AWS exports and worktree directory changes appear as bash/zsh commands to
run after exiting, because the launcher cannot change your parent shell's
environment or directory. With `--print-shell`, a successful AWS profile selection,
worktree creation or worktree `cd` exits the launcher and prints that command to
stdout instead, so a shell function can `eval` it:

```sh
# ~/.zshrc or ~/.bashrc
ct() { local cmd; cmd="$(cli-toolbox tui --print-shell)" || return; [ -n "$cmd" ] && eval "$cmd"; }
```

Only pass `tui --print-shell` output to `eval`; plain `tui` never prints commands.
