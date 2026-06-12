package aws

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

type Profile struct {
	Name     string
	Settings map[string]string
}

func Run(args []string, stdout, stderr io.Writer) error {
	flags := flag.NewFlagSet("aws", flag.ContinueOnError)
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
	case "profile":
		return runProfile(remaining[1:], stdout, stderr)
	case "help":
		printUsage(stdout)
		return nil
	default:
		printUsage(stderr)
		return fmt.Errorf("unknown aws command %q", remaining[0])
	}
}

func runProfile(args []string, stdout, stderr io.Writer) error {
	flags := flag.NewFlagSet("aws profile", flag.ContinueOnError)
	flags.SetOutput(io.Discard)

	var help bool
	flags.BoolVar(&help, "h", false, "Show help")
	flags.BoolVar(&help, "help", false, "Show help")

	if err := flags.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			printProfileUsage(stdout)
			return nil
		}

		printProfileUsage(stderr)
		return err
	}

	if help {
		printProfileUsage(stdout)
		return nil
	}

	if flags.NArg() != 0 {
		printProfileUsage(stderr)
		return fmt.Errorf("aws profile does not accept positional arguments: %s", strings.Join(flags.Args(), " "))
	}

	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("resolve home directory: %w", err)
	}

	profiles, err := LoadProfiles(
		filepath.Join(home, ".aws", "config"),
		filepath.Join(home, ".aws", "credentials"),
	)
	if err != nil {
		return err
	}
	if len(profiles) == 0 {
		return fmt.Errorf("no aws profiles found in %q or %q",
			filepath.Join(home, ".aws", "config"),
			filepath.Join(home, ".aws", "credentials"),
		)
	}

	selected, err := SelectProfile(profiles, stderr)
	if err != nil {
		return err
	}
	if selected == nil {
		return nil
	}

	if err := EnsureSSOLogin(*selected, stderr); err != nil {
		return err
	}

	_, err = fmt.Fprintf(stdout, "export AWS_PROFILE=%s\n", shellQuote(selected.Name))
	return err
}

func LoadProfiles(configPath, credentialsPath string) ([]Profile, error) {
	profiles := map[string]Profile{}

	if err := mergeProfiles(profiles, configPath, configSectionName); err != nil {
		return nil, err
	}
	if err := mergeProfiles(profiles, credentialsPath, credentialsSectionName); err != nil {
		return nil, err
	}

	names := make([]string, 0, len(profiles))
	for name := range profiles {
		names = append(names, name)
	}
	sort.Strings(names)

	result := make([]Profile, 0, len(names))
	for _, name := range names {
		result = append(result, profiles[name])
	}

	return result, nil
}

func mergeProfiles(dst map[string]Profile, path string, mapper func(string) (string, bool)) error {
	sections, err := parseINISections(path, mapper)
	if err != nil {
		return err
	}

	for name, settings := range sections {
		profile := dst[name]
		if profile.Name == "" {
			profile = Profile{
				Name:     name,
				Settings: map[string]string{},
			}
		}

		for key, value := range settings {
			profile.Settings[key] = value
		}

		dst[name] = profile
	}

	return nil
}

func parseINISections(path string, mapper func(string) (string, bool)) (map[string]map[string]string, error) {
	file, err := os.Open(path)
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) {
			return map[string]map[string]string{}, nil
		}

		return nil, fmt.Errorf("open %q: %w", path, err)
	}
	defer file.Close()

	sections := map[string]map[string]string{}
	var current string

	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" || strings.HasPrefix(line, "#") || strings.HasPrefix(line, ";") {
			continue
		}

		if strings.HasPrefix(line, "[") && strings.HasSuffix(line, "]") {
			name := strings.TrimSpace(strings.TrimSuffix(strings.TrimPrefix(line, "["), "]"))
			mapped, ok := mapper(name)
			if !ok {
				current = ""
				continue
			}

			current = mapped
			if _, ok := sections[current]; !ok {
				sections[current] = map[string]string{}
			}
			continue
		}

		if current == "" {
			continue
		}

		key, value, ok := splitKeyValue(line)
		if !ok {
			continue
		}

		sections[current][strings.ToLower(key)] = value
	}

	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("read %q: %w", path, err)
	}

	return sections, nil
}

func splitKeyValue(line string) (string, string, bool) {
	idx := strings.IndexAny(line, "=:")
	if idx <= 0 {
		return "", "", false
	}

	return strings.TrimSpace(line[:idx]), strings.TrimSpace(line[idx+1:]), true
}

func configSectionName(section string) (string, bool) {
	if section == "default" {
		return "default", true
	}

	if !strings.HasPrefix(section, "profile ") {
		return "", false
	}

	name := strings.TrimSpace(strings.TrimPrefix(section, "profile "))
	if name == "" {
		return "", false
	}

	return name, true
}

func credentialsSectionName(section string) (string, bool) {
	section = strings.TrimSpace(section)
	if section == "" {
		return "", false
	}

	return section, true
}

func SelectProfile(profiles []Profile, stderr io.Writer) (*Profile, error) {
	if _, err := exec.LookPath("fzf"); err != nil {
		return nil, fmt.Errorf("fzf is not available on PATH")
	}

	lines := make([]string, 0, len(profiles))
	byName := make(map[string]Profile, len(profiles))
	for _, profile := range profiles {
		lines = append(lines, profile.Name)
		byName[profile.Name] = profile
	}

	cmd := exec.Command("fzf", "--prompt", "aws profile> ", "--height", "~100%", "--reverse")
	cmd.Stdin = strings.NewReader(strings.Join(lines, "\n"))
	cmd.Stderr = stderr

	var selected bytes.Buffer
	cmd.Stdout = &selected

	if err := cmd.Run(); err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && exitErr.ExitCode() == 130 {
			return nil, nil
		}

		return nil, fmt.Errorf("select aws profile with fzf: %w", err)
	}

	name := strings.TrimSpace(selected.String())
	profile, ok := byName[name]
	if !ok {
		return nil, fmt.Errorf("selected aws profile %q was not found", name)
	}

	return &profile, nil
}

func EnsureSSOLogin(profile Profile, stderr io.Writer) error {
	if !isSSOProfile(profile) {
		return nil
	}

	if _, err := exec.LookPath("aws"); err != nil {
		return fmt.Errorf("aws is not available on PATH")
	}

	if hasValidSession(profile.Name) {
		return nil
	}

	fmt.Fprintf(stderr, "Running aws sso login for profile %q...\n", profile.Name)

	cmd := exec.Command("aws", "sso", "login", "--profile", profile.Name)
	cmd.Stdout = stderr
	cmd.Stderr = stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("aws sso login failed for profile %q: %w", profile.Name, err)
	}

	return nil
}

func isSSOProfile(profile Profile) bool {
	for key := range profile.Settings {
		if strings.HasPrefix(key, "sso_") || key == "sso_session" {
			return true
		}
	}

	return false
}

func hasValidSession(profileName string) bool {
	cmd := exec.Command("aws", "sts", "get-caller-identity", "--profile", profileName)
	cmd.Stdout = io.Discard
	cmd.Stderr = io.Discard
	return cmd.Run() == nil
}

func shellQuote(value string) string {
	if value == "" {
		return "''"
	}

	return "'" + strings.ReplaceAll(value, "'", `'\''`) + "'"
}

func printUsage(w io.Writer) {
	fmt.Fprintln(w, "AWS helpers.")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  tool-helper aws <command>")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Commands:")
	fmt.Fprintln(w, "  profile   Select an AWS profile and print an export command")
}

func printProfileUsage(w io.Writer) {
	fmt.Fprintln(w, "Select an AWS profile with fzf and print an export command.")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  tool-helper aws profile [flags]")
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Shell usage:")
	fmt.Fprintln(w, `  eval "$(tool-helper aws profile)"`)
	fmt.Fprintln(w)
	fmt.Fprintln(w, "Flags:")
	fmt.Fprintln(w, "  -h, --help   Show help")
}
