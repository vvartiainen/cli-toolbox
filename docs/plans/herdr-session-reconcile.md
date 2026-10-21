# Reconcile herdr workspaces with the guest's restored session

Status: proposed (2026-10-21). Not implemented.

## Goal

`agent project start` should reuse the herdr workspaces the guest already has,
create workspaces only for worktrees that have none, and close workspaces whose
worktree is gone. A restart of the VM should feel like resuming: the same
workspaces, and agents that continue their conversations, without duplicates.

## Where we are now

- The guest runs `herdr server` from boot as the `herdr-server` user service
  (agent-sandbox). It restores its saved session from
  `/home/agent/.config/herdr/session.json`. That file is in the persistent
  `home.img`, so workspaces, tabs and panes survive a VM stop and start.
- Restored panes start as shells. An agent comes back in its pane only when it
  reports a resume command to herdr. For Copilot, that needs
  `herdr integration install copilot` in the guest; it is kept in `home.img`.
  Whether Copilot actually resumes after a restart has not been checked yet.
- `vm::start` (`src/agent/vm.rs`) calls `herdr_steps` after a fresh boot. For
  every worktree, `herdr::open_machine_workspace` runs
  `herdr --machine <vm> workspace create` and, with `--agent`,
  `herdr --machine <vm> agent start <worktree> --kind <kind>`. It never looks at
  what the guest restored.
- As a result, every restart adds a second workspace per worktree next to the
  restored one, each with its own agent. `agent start` can also fail, because
  agent names must be unique per server and the restored session may already
  have that name.
- `vm::done` removes the worktree but leaves its guest workspace open (known
  gap in docs/agents.md). The restored session brings it back on every boot,
  with a cwd that no longer exists.
- `vm::stop` removes the host herdr machine. That only affects the host
  sidebar; the guest session is untouched and restores on the next start.

## Proposal

### 1. Read the guest's workspaces

Add `herdr::machine_workspaces(machine) -> Vec<MachineWorkspace>` in
`src/herdr/mod.rs`:

```sh
herdr --machine <vm> workspace list
```

The result looks like
`{"result":{"type":"workspace_list","workspaces":[{"workspace_id":"w1","label":"…","agent_status":"…",…}]}}`.
Parse `workspace_id` and `label`. If cwd matching turns out to be needed (see
open questions), `herdr --machine <vm> pane list` gives each pane's `cwd`,
`workspace_id` and `agent`.

Use `exec::run_captured` and parse stdout only, like `worktree_workspace`.
Add a unit test with a captured response, like `finds_a_saved_machine_by_label`.

### 2. Reconcile in `herdr_steps`

The key is the label: cli-toolbox already labels each workspace with the
worktree name, and worktree names are unique per project.

For each worktree:

- **Has a workspace** with its label: keep it. Report
  `Step::Skipped("herdr: workspace <id> restored for <worktree>")`. Do not
  start an agent in it (see open questions for restored panes without one).
- **No workspace:** call `open_machine_workspace` as today, with `--agent` if
  given.

For each guest workspace whose label matches no current worktree, and that
looks like one of ours (see open questions), close it:
`herdr --machine <vm> workspace close <id>`. Report
`Step::Done("herdr: closed workspace <id> of removed worktree <label>")`.

Workspaces the user made by hand, with labels that are not worktree names,
are left alone.

### 3. Run it on every start

`vm::start` takes a different path when the VM is already running: it only
calls `herdr_add` and returns. Run the same reconcile there after `herdr_add`,
so `project start` on a running VM also opens workspaces for worktrees added
since. `restart` gets it for free, since it calls `start`.

### 4. Close the workspace in `done`

In `vm::done`, before removing the worktree and while the VM is running, find
the worktree's workspace by label and close it on the machine. This removes
the item in the docs/agents.md known gaps ("done does not close the
worktree's herdr workspace in the guest").

### 5. `--fresh` for a clean slate

Add `--fresh` to `BootArgs` (`src/cli/agent.rs`) and `VmOptions`. With it,
`herdr_steps` closes every workspace that belongs to a worktree before
creating new ones, so every agent starts a new conversation. Wire the same
choice into the TUI's start action (`src/tui/actions.rs`) if it is useful there.

## Open questions

- **Does Copilot resume?** Restart a VM with the Copilot integration
  installed and check `herdr --machine <vm> agent list` and the pane itself.
  If it does not resume, a restored workspace whose root pane has no agent
  (`agent: null` in `pane list`) should get `agent start` in that pane,
  as long as the agent name is still free.
- **Name clashes:** `agent start` fails when the agent name is taken. After
  reconcile, the restored agent keeps the name, and cli-toolbox only starts
  agents in new workspaces, so this should not happen. Confirm the error text
  and turn it into a skipped step if it still does.
- **Which workspaces are ours:** matching by label alone could close a
  workspace the user named after a removed worktree. Safer options: only close
  workspaces whose label has the `<repo>-agent-wt` prefix, or whose panes'
  cwd is under `/workspace/<label>`. The prefix check is simpler; choose
  one before implementing.
- **Renamed workspaces:** if the user renames a workspace in herdr, the label
  no longer matches and a duplicate is created. Matching on the root pane's
  cwd (`/workspace/<worktree>`) as a fallback would cover that.

## Tests

- Unit tests for parsing `workspace list` and `pane list` responses.
- A unit test for the reconcile decision itself: given worktrees and guest
  workspaces, return which to keep, create and close. Keep it a pure function
  so it needs no herdr.
- By hand, with a running project: start, restart, `add --restart`, `done`,
  and `start --fresh`, checking `herdr --machine <vm> workspace list` after
  each.
