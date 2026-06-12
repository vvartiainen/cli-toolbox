package main

import (
	"bytes"
	"strings"
	"testing"
)

func TestRunPrintsRootUsageWithoutArgs(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := run(nil, &stdout, &stderr); err != nil {
		t.Fatalf("run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "tool-helper <command>") {
		t.Fatalf("stdout missing root usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsKittySessionHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := run([]string{"kitty-session", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "tool-helper kitty-session [flags]") {
		t.Fatalf("stdout missing kitty-session usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunPrintsAWSProfileHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := run([]string{"aws", "profile", "--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), `eval "$(tool-helper aws profile)"`) {
		t.Fatalf("stdout missing aws profile usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunRejectsKittySessionPositionalArgs(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := run([]string{"kitty-session", "extra"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("run returned nil error")
	}

	if !strings.Contains(err.Error(), "does not accept positional arguments") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "tool-helper kitty-session [flags]") {
		t.Fatalf("stderr missing kitty-session usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunRejectsUnknownKittySessionFlag(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := run([]string{"kitty-session", "--wat"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("run returned nil error")
	}

	if !strings.Contains(err.Error(), "flag provided but not defined") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "tool-helper kitty-session [flags]") {
		t.Fatalf("stderr missing kitty-session usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func TestRunRejectsUnknownAWSSubcommand(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := run([]string{"aws", "wat"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("run returned nil error")
	}

	if !strings.Contains(err.Error(), `unknown aws command "wat"`) {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "tool-helper aws <command>") {
		t.Fatalf("stderr missing aws usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}
