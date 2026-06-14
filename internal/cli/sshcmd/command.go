package sshcmd

import (
	"fmt"
	"io"
	"os"

	toolboxssh "cli-toolbox/internal/ssh"
)

type Command struct {
	Stdout io.Writer `kong:"-"`
	Stderr io.Writer `kong:"-"`
}

func New(stdout, stderr io.Writer) Command {
	return Command{
		Stdout: stdout,
		Stderr: stderr,
	}
}

func (c Command) Run() error {
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("resolve home directory: %w", err)
	}

	return toolboxssh.SelectAndConnectHost(home, c.Stdout, c.Stderr)
}
