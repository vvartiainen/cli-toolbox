package awscmd

import (
	"fmt"
	"io"
	"os"

	"cli-toolbox/internal/aws"
)

type Command struct {
	Stdout  io.Writer  `kong:"-"`
	Stderr  io.Writer  `kong:"-"`
	Profile ProfileCmd `cmd:"" help:"Select an AWS profile and print an export command."`
}

func New(stdout, stderr io.Writer) Command {
	return Command{
		Stdout: stdout,
		Stderr: stderr,
		Profile: ProfileCmd{
			Stdout: stdout,
			Stderr: stderr,
		},
	}
}

type ProfileCmd struct {
	Stdout io.Writer `kong:"-"`
	Stderr io.Writer `kong:"-"`
}

func (ProfileCmd) Help() string {
	return `Shell usage:

  eval "$(cli-toolbox aws profile)"`
}

func (c ProfileCmd) Run() error {
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("resolve home directory: %w", err)
	}

	return aws.SelectAndExportProfile(home, c.Stdout, c.Stderr)
}
