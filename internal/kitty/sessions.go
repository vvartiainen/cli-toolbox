package kitty

import (
	"bytes"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

const sessionGlob = "*.kitty-session"

type choice struct {
	Label string
	Path  string
}

func Run(args []string, stdout, stderr io.Writer) error {
	flags := flag.NewFlagSet("kitty-session", flag.ContinueOnError)
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

	if flags.NArg() != 0 {
		printUsage(stderr)
		return fmt.Errorf("kitty-session does not accept positional arguments: %s", strings.Join(flags.Args(), " "))
	}

	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("resolve home directory: %w", err)
	}

	paths, err := FindSessions(home)
	if err != nil {
		return err
	}
	if len(paths) == 0 {
		return fmt.Errorf("no kitty session files found matching %q", filepath.Join(home, sessionGlob))
	}

	selected, err := SelectSession(home, paths, stderr)
	if err != nil {
		return err
	}
	if selected == "" {
		return nil
	}

	return LaunchSession(selected, stdout, stderr)
}

func FindSessions(home string) ([]string, error) {
	pattern := filepath.Join(home, sessionGlob)

	matches, err := filepath.Glob(pattern)
	if err != nil {
		return nil, fmt.Errorf("glob kitty session files: %w", err)
	}

	sort.Strings(matches)
	return matches, nil
}

func BuildChoices(home string, paths []string) []choice {
	choices := make([]choice, 0, len(paths))

	for _, path := range paths {
		rel, err := filepath.Rel(home, path)
		label := path
		if err == nil && rel != "." && !strings.HasPrefix(rel, "..") {
			label = filepath.Join("~", rel)
		}

		choices = append(choices, choice{
			Label: label,
			Path:  path,
		})
	}

	return choices
}

func SelectSession(home string, paths []string, stderr io.Writer) (string, error) {
	if _, err := exec.LookPath("fzf"); err != nil {
		return "", fmt.Errorf("fzf is not available on PATH")
	}

	choices := BuildChoices(home, paths)

	lines := make([]string, 0, len(choices))
	byLabel := make(map[string]string, len(choices))
	for _, choice := range choices {
		lines = append(lines, choice.Label)
		byLabel[choice.Label] = choice.Path
	}

	cmd := exec.Command("fzf", "--prompt", "kitty session> ", "--height", "40%", "--reverse")
	cmd.Stdin = strings.NewReader(strings.Join(lines, "\n"))
	cmd.Stderr = stderr

	var selected bytes.Buffer
	cmd.Stdout = &selected

	if err := cmd.Run(); err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && exitErr.ExitCode() == 130 {
			return "", nil
		}

		return "", fmt.Errorf("select kitty session with fzf: %w", err)
	}

	label := strings.TrimSpace(selected.String())
	path, ok := byLabel[label]
	if !ok {
		return "", fmt.Errorf("selected kitty session %q was not found", label)
	}

	return path, nil
}

func LaunchSession(path string, stdout, stderr io.Writer) error {
	cmdName, err := remoteControlExecutable()
	if err != nil {
		return err
	}

	args := []string{"@", "action", "goto_session", path}
	cmd := exec.Command(cmdName, args...)
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("launch kitty session %q via remote control: %w", path, err)
	}

	return nil
}

func printUsage(w io.Writer) {
	fmt.Fprintln(w, "Select a kitty session file with fzf and launch it.")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  tool-helper kitty-session [flags]")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Flags:")
	fmt.Fprintln(w, "  -h, --help   Show help")
}

func remoteControlExecutable() (string, error) {
	for _, name := range []string{"kitten", "kitty"} {
		if _, err := exec.LookPath(name); err == nil {
			return name, nil
		}
	}

	return "", fmt.Errorf("neither kitten nor kitty is available on PATH")
}
