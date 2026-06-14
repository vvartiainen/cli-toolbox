package kittycmd

import (
	"fmt"
	"io"
	"os"

	"cli-toolbox/internal/kitty"
)

type Command struct {
	SelectSession SelectSessionCmd `cmd:"" name:"select-session" help:"Select a kitty session file with fzf and launch it."`
}

func New(stdout, stderr io.Writer) Command {
	return Command{
		SelectSession: SelectSessionCmd{
			Stdout: stdout,
			Stderr: stderr,
		},
	}
}

type SelectSessionCmd struct {
	Stdout io.Writer `kong:"-"`
	Stderr io.Writer `kong:"-"`
}

func (c SelectSessionCmd) Run() error {
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("resolve home directory: %w", err)
	}

	return kitty.SelectAndLaunchSession(home, c.Stdout, c.Stderr)
}
