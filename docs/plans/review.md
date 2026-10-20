# TUI review follow-up: Agents tab, `[`/`]` tabs, worktree manager, shell hook

## Context

Review of the uncommitted TUI rework, which adds an inline viewport, tabs, search and number hotkeys.

The structure is sound:

- `app.rs` is a pure state machine that returns `Effect`s.
- `actions.rs` does the I/O.
- `ui.rs` renders.
- `mod.rs` runs the loop and the terminal handoff.

Tests sit next to each module, and the unit tests can drive the state machine without a terminal.

Weak spots found in review:

- **Tool metadata is spread over many `match` arms.** `category`, `label`, `description`, `picker_title`, `matches_tool`, the Enter dispatch in `handle_picker_key`, and `Context::load`/`execute` each have one. `GitWorktreeCreate` is also special-cased in both `open_selected` and `load`.
- **Breadcrumbs are hard-coded.** `draw_details` and `draw_confirmation` hard-code `Category::Git` (`ui.rs:257`, `ui.rs:292`), so they break once the tools move.
- **List and Cleanup duplicate each other.** They show the same worktree list twice. The details screen only repeats the list label.
- **Shell changes are manual.** AWS exports and `cd` are shown as text to copy, because the TUI can't change the parent shell. `tui::run` ignores `_stdout`, even though the TUI already draws on stderr.
- **The help line is overloaded.** Narrow terminals truncate the home help line, and there is no way to see all the keys.
- **The cleanup confirmation has no y/n.** Lists also lack PageUp/PageDown/Home/End.
- **SSH breaks under a stdout pipe.** `ssh::connect` uses `run_inherited`, so piping stdout would break the interactive SSH session.

User decisions:

- Add an **Agents** tab (hotkey `i`) and drop the Git tab.
- Move the CLI to `agent worktree …` and keep `git agent-worktree` as a hidden alias.
- Include all four UX extras: the worktree manager, y/n plus `?` help, the exit-and-apply shell hook, and paging keys with column labels.

## Changes

### 1. Tabs: Agents replaces Git; `[`/`]` switch tabs (`src/tui/app.rs`)

- Change `Category` to `Aws` (`a`), `Agents` (`i`), `Terminal` (`t`). `Category::ALL` becomes `[Aws, Agents, Terminal]`.
- In `handle_home_key`, add `KeyCode::Char(']')` and `KeyCode::Char('[')` to the existing `offset(1)`/`offset(-1)` arms. Keep Left/Right, Tab, and h/l.
- Extend the `category_hotkeys_are_unique_and_not_navigation_keys` test so the forbidden set also covers `[]?`.

### 2. Worktree tools become one manager (`app.rs`, `actions.rs`)

- Change `ToolId` to `AwsProfile, AgentWorktreeCreate, AgentWorktrees, KittySession, SshConnect`, removing `GitWorktreeList` and `GitWorktreeCleanup`. Labels:
  - "New worktree": create the next sibling worktree and branch.
  - "Worktrees": inspect, cd into, or remove agent worktrees.
- Add a `ToolId::direct_action() -> Option<Action>`. Create returns `Some(Action::CreateWorktree)`. `open_selected` and `Context::load` use it instead of comparing against one ToolId.
- Add new `Action` variants:
  - `EnterWorktree(AgentWorktree)`: outputs `cd '<path>'`.
  - The existing `Cleanup(AgentWorktree)` stays.
- Manager picker keys, added to `handle_picker_key` before `picker_text_key` and only when `tool == AgentWorktrees`:
  - Enter opens `WorktreeDetails`.
  - Ctrl-D opens `ConfirmCleanup`.
  - Ctrl-A runs `Execute(CreateWorktree)`. This uses Ctrl-A rather than Ctrl-N, because Ctrl-N already moves down in `text_key`.
- Details screen (`handle_details_key`):
  - Enter or `c` runs `Execute(EnterWorktree)`.
  - `d` opens `ConfirmCleanup`, with `return_picker` carried over.
  - Esc goes back, and ↑↓/PgUp/PgDn scroll as before.
- Result return (`handle_result_key`): every worktree action (`CreateWorktree`, `Cleanup`, `EnterWorktree`) reloads the `AgentWorktrees` picker. Other results go Home. Add `Action::return_tool()` for this, replacing the `matches!(…, Cleanup)` check.
- In `ui.rs`, the details and confirmation breadcrumbs use `Category::Agents` through `ToolId::AgentWorktrees.category()`, not a hard-coded value. The details hint shows `⏎ cd here · d remove · esc back`.

### 3. Confirmation y/n and the `?`/F1 help overlay

- In `handle_confirm_key`, `y` runs the Remove action and `n` cancels. Tab, ←→, and Enter keep their current behavior.
- Add `App.show_help: bool`.
  - `?` opens the overlay on Home outside search mode, and on Details, Confirm, and Result screens.
  - F1 opens it everywhere, since `?` is text input in pickers.
  - When the overlay is open, any key closes it. That check goes first in `handle_key`, after the Ctrl-C check.
- `ui::draw_help` draws a key list for the current screen inside the same `panel`. It uses two columns so it fits `HEIGHT = 12`.
- Shorten the bottom hints, for example:
  - Home: `a/i/t [ ] tabs · 1-9 run · / search · ? help · q quit`
  - Picker: `type to filter · ⏎ select · F5 refresh · F1 help · esc back`

### 4. Paging keys and column labels

- `text_key` and the home handler gain PageUp/PageDown (±`PAGE` rows, with `const PAGE: usize = 8`, the list height for `HEIGHT`) and Home/End (first and last row). Reuse `move_selection`, changing `delta` to `isize`.
- In `draw_picker`, worktree resources render as aligned columns: the name padded to the widest name, the `[branch]` in the accent color, and the path dimmed. Add `Resource::columns()` for this. Filtering still uses `label()`, so search behavior is unchanged.

### 5. Exit-and-apply shell hook (`src/cli/tui.rs`, `src/tui/mod.rs`, `actions.rs`, `ssh`)

- Add a `--print-shell` flag to `TuiCmd`. The flag's help text: "After an AWS profile or worktree action succeeds, exit and print the shell command to stdout for eval."
- `Outcome` gains `shell: Option<String>`. It is set for:
  - `Aws`: the `aws::export_profile` string.
  - `CreateWorktree` and `EnterWorktree`: `cd <aws::shell_quote(path)>`. Non-UTF-8 paths give `None`, matching `directory_instructions`.
- `tui::run(stdout, …)`:
  - With `--print-shell`, `check_terminals` requires only stdin and stderr.
  - `event_loop` returns `Option<String>`. After `handoff`, when print-shell is on, `!outcome.failed` and `outcome.shell` is `Some`, it returns that command.
  - After the terminal is restored, `run` writes the command and a newline to `stdout`.
  - Without the flag, the behavior stays as it is now: the result screen with instructions.
- `App.print_shell: bool`, used only for hints. For example, the details screen shows `⏎ cd & exit` instead of `⏎ cd here`.
- In `src/ssh/mod.rs`, `connect` switches from `exec::run_inherited` to `exec::run_on_stderr`. That routes the child's stdout to the stderr tty, so `kitten ssh` still gets a terminal when stdout is a pipe. Kitty uses `run_captured`, and AWS SSO already uses `run_on_stderr`, so neither changes.
- README shell function:

  ```sh
  ct() { local cmd; cmd="$(cli-toolbox tui --print-shell)" || return; [ -n "$cmd" ] && eval "$cmd"; }
  ```

### 6. CLI: `agent worktree`, with `git agent-worktree` as a hidden alias

- New `src/cli/agent.rs`: `AgentCmd { Worktree { command: AgentWorktreeCommands } }`.
  - Move the `AgentWorktreeCommands` enum, its run match and `working_dir()` here from `src/cli/git.rs`.
  - Update the `after_help` text to say `cd "$(cli-toolbox agent worktree create)"`.
- `src/cli/git.rs` keeps `GitCmd`/`agent-worktree`, but delegates to `agent::run_worktree(command, …)`.
- In `src/cli/mod.rs`:
  - Add `Agent(AgentCmd)` (/// "AI agent helpers.").
  - Mark `Git` with `#[command(hide = true)]`.
- Domain logic stays in `src/git/mod.rs`, unchanged.
- Tests in `cli/mod.rs`:
  - Copy the agent-worktree help tests to the `agent worktree` paths.
  - Keep one alias test: `git agent-worktree --help` still works.
  - Update the completion command list (line ~360) to include `agent`.
  - Point the completion subcommand test at `agent worktree`.

### 7. Docs

- `README.md`: rewrite the `tui` section:
  - The Agents tab and `i`.
  - `[`/`]`.
  - `?`/F1.
  - The manager keys.
  - `--print-shell` and the `ct` function.
  - Replace "Do not pass tui output to eval" with the `--print-shell` guidance.
- Also in `README.md`: rename the `git agent-worktree` section to `agent worktree` and mention the alias.
- `src/cli/tui.rs` `after_help`: a/i/t, `[`/`]`, `?`.
- `docs/plans/agent-tasks.md`: one-line note that worktrees now live under `agent worktree`.

## Tests (happy path first, beside each module)

- `app.rs`:
  - `]`/`[` cycle `Aws → Agents → Terminal → Aws` and back.
  - `i` `1` gives `Execute(CreateWorktree)`, and `i` `2` gives `Load(AgentWorktrees)`.
  - In the manager, Enter opens details, `c` gives `Execute(EnterWorktree)`, `d` and then `y` give `Execute(Cleanup)`, and Ctrl-D and then `n` return to the picker.
  - A worktree result returns `Load(AgentWorktrees)`, and an AWS result returns Home.
  - `?` toggles `show_help`, and the next key only closes it (no effect).
  - PageDown/End clamp in the picker and on home.
  - Update the existing tests that use `g`/`GitWorktreeCleanup`.
- `actions.rs`: the `Outcome.shell` values, an AWS export and a quoted `cd`, including the apostrophe-path case.
- `mod.rs`:
  - Extend `check_terminals` with a print-shell variant that accepts a non-tty stdout.
  - Factor out a pure `fn shell_exit(print_shell, &Outcome) -> Option<String>` and unit-test it.
- `ui.rs`: the home render contains `i Agents`, the help overlay render contains `[ ]`, and the worktree column render contains the branch and the path.
- `cli/mod.rs`: the help, alias and completion tests above.

## Verification

1. Run `just test`, then `just fmt`. Clippy with `-D warnings` must be clean.
2. Manual pass in this repo:
   - `cargo run -- tui`: `]`/`[` cycle the tabs.
   - `i 1` creates a worktree, and the result returns to the manager list showing columns.
   - Enter, then `c`, shows the cd instructions.
   - Ctrl-D, then `y`, removes it.
   - `?` shows help.
   - `t 2` connects over SSH.
3. Print-shell mode:
   - `cmd="$(cargo run -q -- tui --print-shell)"; echo "$cmd"`. Pick an AWS profile and it prints `export AWS_PROFILE=…`. Choosing "New worktree" prints `cd '…'`.
   - SSH from the same mode still gets an interactive session.
4. Check that `cargo run -- agent worktree list` and `cargo run -- git agent-worktree list` give the same output, and that `cargo run -- --help` does not list `git`.
