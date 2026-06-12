package aws

import (
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestLoadProfilesMergesConfigAndCredentials(t *testing.T) {
	dir := t.TempDir()

	configPath := filepath.Join(dir, "config")
	credentialsPath := filepath.Join(dir, "credentials")

	if err := os.WriteFile(configPath, []byte(`
[default]
region = eu-north-1

[profile sandbox]
sso_start_url = https://example.awsapps.com/start
sso_region = eu-west-1

[sso-session shared]
sso_start_url = ignored
`), 0o644); err != nil {
		t.Fatalf("write config: %v", err)
	}

	if err := os.WriteFile(credentialsPath, []byte(`
[sandbox]
aws_access_key_id = test

[prod]
aws_secret_access_key = secret
`), 0o644); err != nil {
		t.Fatalf("write credentials: %v", err)
	}

	profiles, err := LoadProfiles(configPath, credentialsPath)
	if err != nil {
		t.Fatalf("LoadProfiles returned error: %v", err)
	}

	if got, want := profileNames(profiles), []string{"default", "prod", "sandbox"}; !reflect.DeepEqual(got, want) {
		t.Fatalf("profile names = %v, want %v", got, want)
	}

	sandbox := profiles[2]
	if sandbox.Settings["sso_start_url"] != "https://example.awsapps.com/start" {
		t.Fatalf("sandbox missing config values: %#v", sandbox.Settings)
	}
	if sandbox.Settings["aws_access_key_id"] != "test" {
		t.Fatalf("sandbox missing credentials values: %#v", sandbox.Settings)
	}
}

func TestLoadProfilesMissingFilesIsEmpty(t *testing.T) {
	profiles, err := LoadProfiles("/missing/config", "/missing/credentials")
	if err != nil {
		t.Fatalf("LoadProfiles returned error: %v", err)
	}

	if len(profiles) != 0 {
		t.Fatalf("expected no profiles, got %d", len(profiles))
	}
}

func TestIsSSOProfile(t *testing.T) {
	if !isSSOProfile(Profile{
		Name: "sandbox",
		Settings: map[string]string{
			"sso_session": "shared",
		},
	}) {
		t.Fatal("expected sso profile to be detected")
	}

	if isSSOProfile(Profile{
		Name: "prod",
		Settings: map[string]string{
			"region": "eu-west-1",
		},
	}) {
		t.Fatal("expected non-sso profile")
	}
}

func TestShellQuote(t *testing.T) {
	if got, want := shellQuote("sandbox"), "'sandbox'"; got != want {
		t.Fatalf("shellQuote = %q, want %q", got, want)
	}

	if got, want := shellQuote("dev's account"), "'dev'\\''s account'"; got != want {
		t.Fatalf("shellQuote = %q, want %q", got, want)
	}
}

func profileNames(profiles []Profile) []string {
	names := make([]string, 0, len(profiles))
	for _, profile := range profiles {
		names = append(names, profile.Name)
	}

	return names
}
