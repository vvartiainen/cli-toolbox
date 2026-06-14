package cli

import (
	"bytes"
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
