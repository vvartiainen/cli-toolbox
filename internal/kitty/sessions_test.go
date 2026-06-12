package kitty

import (
	"bytes"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestFindSessionsReturnsSortedMatches(t *testing.T) {
	home := t.TempDir()

	for _, name := range []string{"z.kitty-session", "a.kitty-session", "notes.txt"} {
		path := filepath.Join(home, name)
		if err := os.WriteFile(path, []byte(name), 0o644); err != nil {
			t.Fatalf("write %s: %v", name, err)
		}
	}

	got, err := FindSessions(home)
	if err != nil {
		t.Fatalf("FindSessions returned error: %v", err)
	}

	want := []string{
		filepath.Join(home, "a.kitty-session"),
		filepath.Join(home, "z.kitty-session"),
	}

	if !reflect.DeepEqual(got, want) {
		t.Fatalf("FindSessions mismatch\nwant: %v\ngot:  %v", want, got)
	}
}

func TestBuildChoicesUsesHomeRelativeLabels(t *testing.T) {
	home := t.TempDir()
	paths := []string{
		filepath.Join(home, "alpha.kitty-session"),
		filepath.Join(home, "nested", "beta.kitty-session"),
	}

	got := BuildChoices(home, paths)
	want := []choice{
		{Label: "~/alpha.kitty-session", Path: paths[0]},
		{Label: filepath.Join("~", "nested", "beta.kitty-session"), Path: paths[1]},
	}

	if !reflect.DeepEqual(got, want) {
		t.Fatalf("BuildChoices mismatch\nwant: %#v\ngot:  %#v", want, got)
	}
}

func TestRunPrintsHelp(t *testing.T) {
	var stdout bytes.Buffer
	var stderr bytes.Buffer

	if err := Run([]string{"--help"}, &stdout, &stderr); err != nil {
		t.Fatalf("Run returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), "cli-toolbox kitty-session [flags]") {
		t.Fatalf("stdout missing kitty-session usage: %q", stdout.String())
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

	if !strings.Contains(stderr.String(), "cli-toolbox kitty-session [flags]") {
		t.Fatalf("stderr missing kitty-session usage: %q", stderr.String())
	}

	if stdout.Len() != 0 {
		t.Fatalf("stdout not empty: %q", stdout.String())
	}
}
