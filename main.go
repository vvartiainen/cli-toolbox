package main

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"os"

	"cli-toolbox/internal/aws"
	"cli-toolbox/internal/kitty"
	sshhelper "cli-toolbox/internal/sshhelper"
)

func main() {
	if err := run(os.Args[1:], os.Stdout, os.Stderr); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

func run(args []string, stdout, stderr io.Writer) error {
	flags := flag.NewFlagSet("cli-toolbox", flag.ContinueOnError)
	flags.SetOutput(io.Discard)

	var help bool
	flags.BoolVar(&help, "h", false, "Show help")
	flags.BoolVar(&help, "help", false, "Show help")

	if err := flags.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			printUsage(stdout)
			return nil
		}

		printUsage(stderr)
		return err
	}

	if help {
		printUsage(stdout)
		return nil
	}

	remaining := flags.Args()
	if len(remaining) == 0 {
		printUsage(stdout)
		return nil
	}

	switch remaining[0] {
	case "help":
		printUsage(stdout)
		return nil
	case "aws":
		return aws.Run(remaining[1:], stdout, stderr)
	case "kitty-session":
		return kitty.Run(remaining[1:], stdout, stderr)
	case "ssh":
		return sshhelper.Run(remaining[1:], stdout, stderr)
	default:
		printUsage(stderr)
		return fmt.Errorf("unknown command %q", remaining[0])
	}
}

func printUsage(w io.Writer) {
	fmt.Fprintln(w, "cli-toolbox helps with small workflow tasks.")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  cli-toolbox <command>")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Commands:")
	fmt.Fprintln(w, "  aws             AWS helpers")
	fmt.Fprintln(w, "  kitty-session   Select a kitty session file with fzf and launch it")
	fmt.Fprintln(w, "  ssh             Select an SSH host from config and connect with kitten ssh")
}
