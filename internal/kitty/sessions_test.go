package kitty

import (
	"os"
	"path/filepath"
	"reflect"
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
