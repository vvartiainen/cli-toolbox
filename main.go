package main

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"os"

	"tool-helper/internal/kitty"
)

func main() {
	if err := run(os.Args[1:], os.Stdout, os.Stderr); err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
}

func run(args []string, stdout, stderr io.Writer) error {
	flags := flag.NewFlagSet("tool-helper", flag.ContinueOnError)
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
	case "kitty-session":
		return kitty.Run(remaining[1:], stdout, stderr)
	default:
		printUsage(stderr)
		return fmt.Errorf("unknown command %q", remaining[0])
	}
}

func printUsage(w io.Writer) {
	fmt.Fprintln(w, "tool-helper helps with small workflow tasks.")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  tool-helper <command>")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Commands:")
	fmt.Fprintln(w, "  kitty-session   Select a kitty session file with fzf and launch it")
}
