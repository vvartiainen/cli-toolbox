package sshhelper

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestLoadHostsReadsConfigAndIncludes(t *testing.T) {
	dir := t.TempDir()
	sshDir := filepath.Join(dir, ".ssh")
	if err := os.MkdirAll(filepath.Join(sshDir, "conf.d"), 0o755); err != nil {
		t.Fatalf("create ssh dir: %v", err)
	}

	configPath := filepath.Join(sshDir, "config")
	includePath := filepath.Join(sshDir, "conf.d", "work.conf")

	if err := os.WriteFile(configPath, []byte(`
Host dev prod *.ignored !blocked
  User vvarti

Include conf.d/*.conf
`), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}

	if err := os.WriteFile(includePath, []byte(`
Host bastion
  HostName bastion.example.com

Host "quoted-host"
  HostName quoted.example.com
`), 0o644); err != nil {
		t.Fatalf("write include: %v", err)
	}

	hosts, err := LoadHosts(configPath)
	if err != nil {
		t.Fatalf("LoadHosts returned error: %v", err)
	}

	if got, want := hostNames(hosts), []string{"bastion", "dev", "prod", "quoted-host"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("host names = %v, want %v", got, want)
	}
}

func TestLoadHostsMissingConfigIsEmpty(t *testing.T) {
	hosts, err := LoadHosts("/missing/ssh/config")
	if err != nil {
		t.Fatalf("LoadHosts returned error: %v", err)
	}

	if len(hosts) != 0 {
		t.Fatalf("expected no hosts, got %d", len(hosts))
	}
}

func TestParseDirectiveSupportsEqualsAndComments(t *testing.T) {
	key, args, ok := parseDirective(`Include = "conf.d/*.conf" extra.conf # comment`)
	if !ok {
		t.Fatal("expected directive to parse")
	}

	if key != "include" {
		t.Fatalf("key = %q, want %q", key, "include")
	}

	if got, want := args, []string{"conf.d/*.conf", "extra.conf"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("args = %v, want %v", got, want)
	}
}

func TestRunPrintsHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "cli-toolbox ssh [flags]") {
		t.Fatalf("stdout missing ssh usage: %q", stdout.String())
	}

	if stderr.Len() != 0 {
		t.Fatalf("stderr not empty: %q", stderr.String())
	}
}

func TestRunRejectsPositionalArgs(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	err := Run([]string{"extra"}, &stdout, &stderr)
	if err == nil {
		t.Fatal("Run returned nil error")
	}

	if !strings.Contains(err.Error(), "does not accept positional arguments") {
		t.Fatalf("unexpected error: %v", err)
	}

	if !strings.Contains(stderr.String(), "cli-toolbox ssh [flags]") {
		t.Fatalf("stderr missing ssh usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}

func hostNames(hosts []Host) []string {
	names := make([]string, 0, len(hosts))
	for _, host := range hosts {
		names = append(names, host.Name)
	}

	return names
}
