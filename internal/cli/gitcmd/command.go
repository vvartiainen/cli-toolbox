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
	AgentWorktree AgentWorktreeCmd `cmd:"" name:"agent-worktree" help:"Create a sibling git worktree and branch for AI agents."`
}

func New(stdout, stderr io.Writer) Command {
	return Command{
		Stdout: stdout,
		Stderr: stderr,
		AgentWorktree: AgentWorktreeCmd{
			Stdout: stdout,
			Stderr: stderr,
		},
	}
}

type AgentWorktreeCmd struct {
	Stdout io.Writer `kong:"-"`
	Stderr io.Writer `kong:"-"`
}

func (AgentWorktreeCmd) Help() string {
	return `Creates a new worktree and branch next to the current directory, named <cwd>-agent-wt, and prints the path.

Shell usage:

  cd "$(cli-toolbox git agent-worktree)"`
}

func (c AgentWorktreeCmd) Run() error {
	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("resolve working directory: %w", err)
	}

	return toolboxgit.CreateAgentWorktree(cwd, c.Stdout, c.Stderr)
}
