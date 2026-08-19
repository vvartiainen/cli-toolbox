package cli

import (
	"bytes"
	"strconv"
	"strings"
	"testing"
)

func TestRunPrintsRootUsageWithoutArgs(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run(nil, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox <command>") {
		t.Fatalf("stdout missing root usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsKittyHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"kitty", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox kitty") {
		t.Fatalf("stdout missing kitty usage: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), "select-session") {
		t.Fatalf("stdout missing kitty select-session subcommand: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunRejectsKittyWithoutSubcommand(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"kitty"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "expected") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Usage: cli-toolbox kitty <command>") {
		t.Fatalf("stderr missing kitty usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunPrintsKittySelectSessionHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"kitty", "select-session", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox kitty") || !strings.Contains(stdout.String(), "select-session") {
		t.Fatalf("stdout missing kitty select-session usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsSSHHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"ssh", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox ssh <command>") {
		t.Fatalf("stdout missing ssh usage: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), "connect") {
		t.Fatalf("stdout missing ssh connect subcommand: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsSSHConnectHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"ssh", "connect", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox ssh connect") {
		t.Fatalf("stdout missing ssh connect usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsGitHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"git", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox git <command>") {
		t.Fatalf("stdout missing git usage: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), "agent-worktree") {
		t.Fatalf("stdout missing git agent-worktree subcommand: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsGitAgentWorktreeHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"git", "agent-worktree", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox git agent-worktree") {
		t.Fatalf("stdout missing git agent-worktree usage: %q", stdout.String())
	}

	for _, subcommand := range []string{"create", "list", "cleanup"} {
		if !strings.Contains(stdout.String(), subcommand) {
			t.Fatalf("stdout missing git agent-worktree %s subcommand: %q", subcommand, stdout.String())
		}
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunRejectsGitAgentWorktreeWithoutSubcommand(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"git", "agent-worktree"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "expected") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Usage: cli-toolbox git agent-worktree <command>") {
		t.Fatalf("stderr missing git agent-worktree usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunPrintsGitAgentWorktreeCreateHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"git", "agent-worktree", "create", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox git agent-worktree create") {
		t.Fatalf("stdout missing git agent-worktree create usage: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), `cd "$(cli-toolbox git agent-worktree create)"`) {
		t.Fatalf("stdout missing git agent-worktree create shell usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsGitAgentWorktreeListHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"git", "agent-worktree", "list", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox git agent-worktree list") {
		t.Fatalf("stdout missing git agent-worktree list usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsGitAgentWorktreeCleanupHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"git", "agent-worktree", "cleanup", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox git agent-worktree cleanup") {
		t.Fatalf("stdout missing git agent-worktree cleanup usage: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), "--all") {
		t.Fatalf("stdout missing git agent-worktree cleanup --all flag: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunRejectsUnknownGitSubcommand(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"git", "wat"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "expected") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Usage: cli-toolbox git <command>") {
		t.Fatalf("stderr missing git usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunPrintsAWSProfileHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"aws", "profile", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox aws profile") {
		t.Fatalf("stdout missing aws profile usage: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), `eval "$(cli-toolbox aws profile)"`) {
		t.Fatalf("stdout missing aws profile shell usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunRejectsKittySelectSessionPositionalArgs(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"kitty", "select-session", "extra"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "unexpected argument") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Usage: cli-toolbox kitty") || !strings.Contains(stderr.String(), "select-session") {
		t.Fatalf("stderr missing kitty select-session usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunRejectsUnknownKittySelectSessionFlag(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"kitty", "select-session", "--wat"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "unknown flag") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Usage: cli-toolbox kitty") || !strings.Contains(stderr.String(), "select-session") {
		t.Fatalf("stderr missing kitty select-session usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunRejectsUnknownAWSSubcommand(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"aws", "wat"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "expected") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Usage: cli-toolbox aws <command>") {
		t.Fatalf("stderr missing aws usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunRejectsUnknownSSHSubcommand(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"ssh", "wat"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "expected") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "Usage: cli-toolbox ssh <command>") {
		t.Fatalf("stderr missing ssh usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunPrintsCompletionHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"completion", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "Usage: cli-toolbox completion") {
		t.Fatalf("stdout missing completion usage: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), "--code") {
		t.Fatalf("stdout missing --code flag: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), "tab completion") {
		t.Fatalf("stdout missing completion description: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsZshCompletionScript(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"completion", "-c", "zsh"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "#compdef cli-toolbox") {
		t.Fatalf("stdout missing zsh #compdef: %q", stdout.String())
	}

	if !strings.Contains(stdout.String(), "complete") || !strings.Contains(stdout.String(), "-C") {
		t.Fatalf("stdout missing complete -C hook: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunCompletesRootCommands(t *testing.T) {
	stdout, stderr := completeLine(t, "cli-toolbox ")

	for _, command := range []string{"aws", "completion", "git", "kitty", "ssh"} {
		if !containsLine(stdout, command) {
			t.Fatalf("stdout missing %s: %q", command, stdout)
		}
	}

	if stderr != "" {
		t.Fatalf("stderr not empty: %q", stderr)
	}
}

func TestRunCompletesCompletionShells(t *testing.T) {
	stdout, stderr := completeLine(t, "cli-toolbox completion ")

	for _, shell := range []string{"bash", "zsh", "fish"} {
		if !containsLine(stdout, shell) {
			t.Fatalf("stdout missing %s: %q", shell, stdout)
		}
	}

	if stderr != "" {
		t.Fatalf("stderr not empty: %q", stderr)
	}
}

func TestRunCompletesGitAgentWorktreeSubcommands(t *testing.T) {
	stdout, stderr := completeLine(t, "cli-toolbox git agent-worktree ")

	for _, command := range []string{"create", "list", "cleanup"} {
		if !containsLine(stdout, command) {
			t.Fatalf("stdout missing %s: %q", command, stdout)
		}
	}

	if stderr != "" {
		t.Fatalf("stderr not empty: %q", stderr)
	}
}

func TestRunCompletesCleanupFlags(t *testing.T) {
	stdout, stderr := completeLine(t, "cli-toolbox git agent-worktree cleanup -")

	if !containsLine(stdout, "--all") {
		t.Fatalf("stdout missing --all: %q", stdout)
	}

	if !containsLine(stdout, "--help") {
		t.Fatalf("stdout missing --help: %q", stdout)
	}

	if stderr != "" {
		t.Fatalf("stderr not empty: %q", stderr)
	}
}

func TestRunRejectsUnknownCompletionShell(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"completion", "powershell"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "powershell") && !strings.Contains(err.Error(), "must be one of") {
		t.Fatalf("unexpected error: %v", err)
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func completeLine(t *testing.T, line string) (stdout, stderr string) {
	t.Helper()
	t.Setenv("COMP_LINE", line)
	t.Setenv("COMP_POINT", strconv.Itoa(len(line)))

	var outBuf bytes.Buffer
	var errBuf bytes.Buffer

	if err := Run(nil, &outBuf, &errBuf); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	return outBuf.String(), errBuf.String()
}

func containsLine(output, line string) bool {
	for _, got := range strings.Split(strings.TrimSpace(output), "\n") {
		if got == line {
			return true
		}
	}

	return false
}
