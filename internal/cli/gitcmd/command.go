package gitcmd

import (
	"fmt"
	"io"
	"os"

	toolboxgit "cli-toolbox/internal/git"
)

type Command struct {
	Stdout        io.Writer        `kong:"-"`
	Stderr        io.Writer        `kong:"-"`
	AgentWorktree AgentWorktreeCmd `cmd:"" name:"agent-worktree" help:"Manage sibling git worktrees for AI agents."`
}

func New(stdout, stderr io.Writer) Command {
	return Command{
		Stdout: stdout,
		Stderr: stderr,
		AgentWorktree: AgentWorktreeCmd{
			Create: AgentWorktreeCreateCmd{
				Stdout: stdout,
				Stderr: stderr,
			},
			List: AgentWorktreeListCmd{
				Stdout: stdout,
				Stderr: stderr,
			},
			Cleanup: AgentWorktreeCleanupCmd{
				Stdout: stdout,
				Stderr: stderr,
			},
		},
	}
}

type AgentWorktreeCmd struct {
	Create  AgentWorktreeCreateCmd  `cmd:"" help:"Create a sibling git worktree and branch for AI agents."`
	List    AgentWorktreeListCmd    `cmd:"" help:"List agent worktrees for the current directory."`
	Cleanup AgentWorktreeCleanupCmd `cmd:"" help:"Remove an agent worktree and its branch."`
}

func (AgentWorktreeCmd) Help() string {
	return `Manage sibling git worktrees for AI agents.

Worktrees are created next to the current directory as <cwd>-agent-wt, <cwd>-agent-wt-2, and so on.`
}

type AgentWorktreeCreateCmd struct {
	Stdout io.Writer `kong:"-"`
	Stderr io.Writer `kong:"-"`
}

func (AgentWorktreeCreateCmd) Help() string {
	return `Creates a new worktree and branch next to the current directory and prints the path.

Shell usage:

  cd "$(cli-toolbox git agent-worktree create)"`
}

func (c AgentWorktreeCreateCmd) Run() error {
	cwd, err := workingDir()
	if err != nil {
		return err
	}

	return toolboxgit.CreateAgentWorktree(cwd, c.Stdout, c.Stderr)
}

type AgentWorktreeListCmd struct {
	Stdout io.Writer `kong:"-"`
	Stderr io.Writer `kong:"-"`
}

func (c AgentWorktreeListCmd) Run() error {
	cwd, err := workingDir()
	if err != nil {
		return err
	}

	return toolboxgit.ListAgentWorktrees(cwd, c.Stdout, c.Stderr)
}

type AgentWorktreeCleanupCmd struct {
	Stdout io.Writer `kong:"-"`
	Stderr io.Writer `kong:"-"`
	Name   string    `arg:"" optional:"" name:"name" help:"Worktree name or path to remove." completion-predictor:"worktree"`
	All    bool      `help:"Remove all agent worktrees for the current directory."`
}

func (AgentWorktreeCleanupCmd) Help() string {
	return `Removes an agent worktree and its branch.

If name is omitted and only one agent worktree exists, that one is removed.`
}

func (c AgentWorktreeCleanupCmd) Run() error {
	cwd, err := workingDir()
	if err != nil {
		return err
	}

	return toolboxgit.CleanupAgentWorktree(cwd, c.Name, c.All, c.Stdout, c.Stderr)
}

func workingDir() (string, error) {
	cwd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("resolve working directory: %w", err)
	}

	return cwd, nil
}
