package sshhelper

import (
	"bufio"
	"bytes"
	"errors"
	"flag"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

type Host struct {
	Name string
}

func Run(args []string, stdout, stderr io.Writer) error {
	flags := flag.NewFlagSet("ssh", flag.ContinueOnError)
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
		return fmt.Errorf("ssh does not accept positional arguments: %s", strings.Join(flags.Args(), " "))
	}

	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("resolve home directory: %w", err)
	}

	configPath := filepath.Join(home, ".ssh", "config")
	hosts, err := LoadHosts(configPath)
	if err != nil {
		return err
	}
	if len(hosts) == 0 {
		return fmt.Errorf("no ssh hosts found in %q", configPath)
	}

	selected, err := SelectHost(hosts, stderr)
	if err != nil {
		return err
	}
	if selected == nil {
		return nil
	}

	return Connect(*selected, stdout, stderr)
}

func LoadHosts(configPath string) ([]Host, error) {
	loader := hostLoader{
		seen:  map[string]struct{}{},
		hosts: map[string]Host{},
	}

	if err := loader.loadFile(configPath); err != nil {
		return nil, err
	}

	names := make([]string, 0, len(loader.hosts))
	for name := range loader.hosts {
		names = append(names, name)
	}
	sort.Strings(names)

	hosts := make([]Host, 0, len(names))
	for _, name := range names {
		hosts = append(hosts, loader.hosts[name])
	}

	return hosts, nil
}

type hostLoader struct {
	seen  map[string]struct{}
	hosts map[string]Host
}

func (l *hostLoader) loadFile(path string) error {
	cleanPath := filepath.Clean(path)
	if _, ok := l.seen[cleanPath]; ok {
		return nil
	}
	l.seen[cleanPath] = struct{}{}

	file, err := os.Open(cleanPath)
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) {
			return nil
		}

		return fmt.Errorf("open %q: %w", cleanPath, err)
	}
	defer file.Close()

	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		key, args, ok := parseDirective(scanner.Text())
		if !ok {
			continue
		}

		switch key {
		case "host":
			for _, pattern := range args {
				if !isConcreteHostPattern(pattern) {
					continue
				}
				if _, exists := l.hosts[pattern]; !exists {
					l.hosts[pattern] = Host{Name: pattern}
				}
			}
		case "include":
			for _, includePath := range args {
				if err := l.loadIncludes(filepath.Dir(cleanPath), includePath); err != nil {
					return err
				}
			}
		}
	}

	if err := scanner.Err(); err != nil {
		return fmt.Errorf("read %q: %w", cleanPath, err)
	}

	return nil
}

func (l *hostLoader) loadIncludes(baseDir, pattern string) error {
	pattern = expandHome(pattern)
	if !filepath.IsAbs(pattern) {
		pattern = filepath.Join(baseDir, pattern)
	}

	matches, err := filepath.Glob(pattern)
	if err != nil {
		return fmt.Errorf("glob include %q: %w", pattern, err)
	}

	sort.Strings(matches)
	for _, match := range matches {
		if err := l.loadFile(match); err != nil {
			return err
		}
	}

	return nil
}

func parseDirective(line string) (string, []string, bool) {
	line = strings.TrimSpace(line)
	if line == "" || strings.HasPrefix(line, "#") {
		return "", nil, false
	}

	keyEnd := 0
	for keyEnd < len(line) {
		ch := line[keyEnd]
		if ch == ' ' || ch == '\t' || ch == '=' {
			break
		}
		keyEnd++
	}
	if keyEnd == 0 {
		return "", nil, false
	}

	key := strings.ToLower(line[:keyEnd])
	rest := strings.TrimSpace(line[keyEnd:])
	if strings.HasPrefix(rest, "=") {
		rest = strings.TrimSpace(rest[1:])
	}

	args := splitSSHFields(rest)
	if len(args) == 0 {
		return "", nil, false
	}

	return key, args, true
}

func splitSSHFields(input string) []string {
	fields := []string{}
	var current strings.Builder
	var quote byte
	escaped := false

	flush := func() {
		if current.Len() == 0 {
			return
		}
		fields = append(fields, current.String())
		current.Reset()
	}

	for i := 0; i < len(input); i++ {
		ch := input[i]

		if escaped {
			current.WriteByte(ch)
			escaped = false
			continue
		}

		if ch == '\\' {
			escaped = true
			continue
		}

		if quote != 0 {
			if ch == quote {
				quote = 0
				continue
			}
			current.WriteByte(ch)
			continue
		}

		switch ch {
		case '\'', '"':
			quote = ch
		case '#':
			flush()
			return fields
		case ' ', '\t':
			flush()
		default:
			current.WriteByte(ch)
		}
	}

	flush()
	return fields
}

func isConcreteHostPattern(pattern string) bool {
	if pattern == "" || strings.HasPrefix(pattern, "!") {
		return false
	}

	return !strings.ContainsAny(pattern, "*?")
}

func expandHome(path string) string {
	if path != "~" && !strings.HasPrefix(path, "~/") {
		return path
	}

	home, err := os.UserHomeDir()
	if err != nil {
		return path
	}

	if path == "~" {
		return home
	}

	return filepath.Join(home, path[2:])
}

func SelectHost(hosts []Host, stderr io.Writer) (*Host, error) {
	if _, err := exec.LookPath("fzf"); err != nil {
		return nil, fmt.Errorf("fzf is not available on PATH")
	}

	lines := make([]string, 0, len(hosts))
	byName := make(map[string]Host, len(hosts))
	for _, host := range hosts {
		lines = append(lines, host.Name)
		byName[host.Name] = host
	}

	cmd := exec.Command("fzf", "--prompt", "ssh host> ", "--height", "~100%", "--reverse")
	cmd.Stdin = strings.NewReader(strings.Join(lines, "\n"))
	cmd.Stderr = stderr

	var selected bytes.Buffer
	cmd.Stdout = &selected

	if err := cmd.Run(); err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && exitErr.ExitCode() == 130 {
			return nil, nil
		}

		return nil, fmt.Errorf("select ssh host with fzf: %w", err)
	}

	name := strings.TrimSpace(selected.String())
	host, ok := byName[name]
	if !ok {
		return nil, fmt.Errorf("selected ssh host %q was not found", name)
	}

	return &host, nil
}

func Connect(host Host, stdout, stderr io.Writer) error {
	cmdName, args, err := kittenSSHCommand(host.Name)
	if err != nil {
		return err
	}

	cmd := exec.Command(cmdName, args...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("connect to ssh host %q with kitten ssh: %w", host.Name, err)
	}

	return nil
}

func kittenSSHCommand(host string) (string, []string, error) {
	if _, err := exec.LookPath("kitten"); err == nil {
		return "kitten", []string{"ssh", host}, nil
	}

	if _, err := exec.LookPath("kitty"); err == nil {
		return "kitty", []string{"+kitten", "ssh", host}, nil
	}

	return "", nil, fmt.Errorf("neither kitten nor kitty is available on PATH")
}

func printUsage(w io.Writer) {
	fmt.Fprintln(w, "Select an SSH host from config and connect with kitten ssh.")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  cli-toolbox ssh [flags]")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Flags:")
	fmt.Fprintln(w, "  -h, --help   Show help")
}
