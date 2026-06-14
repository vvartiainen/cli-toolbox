package ssh

import (
	"os"
	"path/filepath"
	"reflect"
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

func hostNames(hosts []Host) []string {
	names := make([]string, 0, len(hosts))
	for _, host := range hosts {
		names = append(names, host.Name)
	}

	return names
}
