package git

import (
	"bytes"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func TestNextAgentWorktreeNameFillsGaps(t *testing.T) {
	if got, want := nextAgentWorktreeName("cli-toolbox", nil), "cli-toolbox-agent-wt"; got != want {
		t.Fatalf("next name = %q, want %q", got, want)
	}

	used := map[string]struct{}{
		"cli-toolbox-agent-wt":   {},
		"cli-toolbox-agent-wt-2": {},
	}
	if got, want := nextAgentWorktreeName("cli-toolbox", used), "cli-toolbox-agent-wt-3"; got != want {
		t.Fatalf("next name = %q, want %q", got, want)
	}

	used = map[string]struct{}{
		"cli-toolbox-agent-wt-2": {},
	}
	if got, want := nextAgentWorktreeName("cli-toolbox", used), "cli-toolbox-agent-wt"; got != want {
		t.Fatalf("next name = %q, want %q", got, want)
	}
}

func TestIsAgentWorktreeName(t *testing.T) {
	tests := []struct {
		name string
		want bool
	}{
		{name: "cli-toolbox-agent-wt", want: true},
		{name: "cli-toolbox-agent-wt-2", want: true},
		{name: "cli-toolbox-agent-wt-10", want: true},
		{name: "cli-toolbox-agent-wt-1", want: true},
		{name: "cli-toolbox", want: false},
		{name: "cli-toolbox-agent-wt-02", want: false},
		{name: "cli-toolbox-agent-wt-extra", want: false},
		{name: "other-agent-wt", want: false},
	}

	for _, test := range tests {
		if got := isAgentWorktreeName("cli-toolbox", test.name); got != test.want {
			t.Fatalf("isAgentWorktreeName(%q) = %v, want %v", test.name, got, test.want)
		}
	}
}

func TestCreateAgentWorktreeCreatesSiblingWorktreeAndBranch(t *testing.T) {
	skipWithoutGit(t)

	root := t.TempDir()
	repo := filepath.Join(root, "demo")
	initGitRepo(t, repo)

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	if err := CreateAgentWorktree(repo, &stdout, &stderr); err != nil {
		t.Fatalf("CreateAgentWorktree returned error: %v\nstderr: %s", err, stderr.String())
	}

	wantPath := filepath.Join(root, "demo-agent-wt")
	assertSamePath(t, strings.TrimSpace(stdout.String()), wantPath)

	if _, err := os.Stat(wantPath); err != nil {
		t.Fatalf("worktree path missing: %v", err)
	}

	if got, want := mustGitOutput(t, wantPath, "rev-parse", "--abbrev-ref", "HEAD"), "demo-agent-wt"; got != want {
		t.Fatalf("worktree branch = %q, want %q", got, want)
	}

	assertWorktreeListed(t, repo, wantPath, "demo-agent-wt")
}

func TestCreateAgentWorktreeAllocatesNextName(t *testing.T) {
	skipWithoutGit(t)

	root := t.TempDir()
	repo := filepath.Join(root, "demo")
	initGitRepo(t, repo)

	var stderr bytes.Buffer
	if err := CreateAgentWorktree(repo, io.Discard, &stderr); err != nil {
		t.Fatalf("first create: %v\nstderr: %s", err, stderr.String())
	}

	var stdout bytes.Buffer
	if err := CreateAgentWorktree(repo, &stdout, &stderr); err != nil {
		t.Fatalf("second create: %v\nstderr: %s", err, stderr.String())
	}

	wantPath := filepath.Join(root, "demo-agent-wt-2")
	assertSamePath(t, strings.TrimSpace(stdout.String()), wantPath)

	if got, want := mustGitOutput(t, wantPath, "rev-parse", "--abbrev-ref", "HEAD"), "demo-agent-wt-2"; got != want {
		t.Fatalf("worktree branch = %q, want %q", got, want)
	}
}

func TestListAgentWorktrees(t *testing.T) {
	skipWithoutGit(t)

	root := t.TempDir()
	repo := filepath.Join(root, "demo")
	initGitRepo(t, repo)

	var stderr bytes.Buffer
	if err := CreateAgentWorktree(repo, io.Discard, &stderr); err != nil {
		t.Fatalf("first create: %v\nstderr: %s", err, stderr.String())
	}
	if err := CreateAgentWorktree(repo, io.Discard, &stderr); err != nil {
		t.Fatalf("second create: %v\nstderr: %s", err, stderr.String())
	}

	worktrees, err := AgentWorktrees(repo)
	if err != nil {
		t.Fatalf("AgentWorktrees returned error: %v", err)
	}

	gotNames := make([]string, 0, len(worktrees))
	gotBranches := make([]string, 0, len(worktrees))
	for _, wt := range worktrees {
		gotNames = append(gotNames, wt.Name)
		gotBranches = append(gotBranches, wt.Branch)
	}

	if want := []string{"demo-agent-wt", "demo-agent-wt-2"}; !reflect.DeepEqual(gotNames, want) {
		t.Fatalf("names = %v, want %v", gotNames, want)
	}
	if want := []string{"demo-agent-wt", "demo-agent-wt-2"}; !reflect.DeepEqual(gotBranches, want) {
		t.Fatalf("branches = %v, want %v", gotBranches, want)
	}

	var stdout bytes.Buffer
	if err := ListAgentWorktrees(repo, &stdout, &stderr); err != nil {
		t.Fatalf("ListAgentWorktrees returned error: %v", err)
	}

	got := strings.Split(strings.TrimSpace(stdout.String()), "\n")
	want := []string{
		filepath.Join(root, "demo-agent-wt"),
		filepath.Join(root, "demo-agent-wt-2"),
	}
	if len(got) != len(want) {
		t.Fatalf("list stdout = %v, want %v", got, want)
	}
	for i := range want {
		assertSamePath(t, got[i], want[i])
	}
}

func TestCleanupAgentWorktreeRemovesNamedWorktree(t *testing.T) {
	skipWithoutGit(t)

	root := t.TempDir()
	repo := filepath.Join(root, "demo")
	initGitRepo(t, repo)

	var stderr bytes.Buffer
	if err := CreateAgentWorktree(repo, io.Discard, &stderr); err != nil {
		t.Fatalf("first create: %v\nstderr: %s", err, stderr.String())
	}
	if err := CreateAgentWorktree(repo, io.Discard, &stderr); err != nil {
		t.Fatalf("second create: %v\nstderr: %s", err, stderr.String())
	}

	var stdout bytes.Buffer
	if err := CleanupAgentWorktree(repo, "demo-agent-wt", false, &stdout, &stderr); err != nil {
		t.Fatalf("CleanupAgentWorktree returned error: %v\nstderr: %s", err, stderr.String())
	}

	wantPath := filepath.Join(root, "demo-agent-wt")
	assertSamePath(t, strings.TrimSpace(stdout.String()), wantPath)

	if _, err := os.Stat(wantPath); !os.IsNotExist(err) {
		t.Fatalf("worktree path still exists: %v", err)
	}

	remaining, err := AgentWorktrees(repo)
	if err != nil {
		t.Fatalf("AgentWorktrees returned error: %v", err)
	}
	if len(remaining) != 1 || remaining[0].Name != "demo-agent-wt-2" {
		t.Fatalf("remaining worktrees = %#v, want demo-agent-wt-2", remaining)
	}

	branchSet := map[string]struct{}{}
	for _, branch := range strings.Split(mustGitOutput(t, repo, "for-each-ref", "--format=%(refname:short)", "refs/heads"), "\n") {
		branchSet[branch] = struct{}{}
	}
	if _, ok := branchSet["demo-agent-wt"]; ok {
		t.Fatalf("deleted branch still present: %v", branchSet)
	}
	if _, ok := branchSet["demo-agent-wt-2"]; !ok {
		t.Fatalf("kept branch missing: %v", branchSet)
	}
}

func TestCleanupAgentWorktreeRemovesOnlyWorktreeWhenUnnamed(t *testing.T) {
	skipWithoutGit(t)

	root := t.TempDir()
	repo := filepath.Join(root, "demo")
	initGitRepo(t, repo)

	var stderr bytes.Buffer
	if err := CreateAgentWorktree(repo, io.Discard, &stderr); err != nil {
		t.Fatalf("create: %v\nstderr: %s", err, stderr.String())
	}

	if err := CleanupAgentWorktree(repo, "", false, io.Discard, &stderr); err != nil {
		t.Fatalf("CleanupAgentWorktree returned error: %v\nstderr: %s", err, stderr.String())
	}

	remaining, err := AgentWorktrees(repo)
	if err != nil {
		t.Fatalf("AgentWorktrees returned error: %v", err)
	}
	if len(remaining) != 0 {
		t.Fatalf("expected no worktrees, got %#v", remaining)
	}
}

func TestCleanupAllAgentWorktrees(t *testing.T) {
	skipWithoutGit(t)

	root := t.TempDir()
	repo := filepath.Join(root, "demo")
	initGitRepo(t, repo)

	var stderr bytes.Buffer
	if err := CreateAgentWorktree(repo, io.Discard, &stderr); err != nil {
		t.Fatalf("first create: %v\nstderr: %s", err, stderr.String())
	}
	if err := CreateAgentWorktree(repo, io.Discard, &stderr); err != nil {
		t.Fatalf("second create: %v\nstderr: %s", err, stderr.String())
	}

	if err := CleanupAgentWorktree(repo, "", true, io.Discard, &stderr); err != nil {
		t.Fatalf("CleanupAgentWorktree returned error: %v\nstderr: %s", err, stderr.String())
	}

	remaining, err := AgentWorktrees(repo)
	if err != nil {
		t.Fatalf("AgentWorktrees returned error: %v", err)
	}
	if len(remaining) != 0 {
		t.Fatalf("expected no worktrees, got %#v", remaining)
	}
}

func skipWithoutGit(t *testing.T) {
	t.Helper()

	if _, err := exec.LookPath("git"); err != nil {
		t.Skip("git is not available on PATH")
	}
}

func initGitRepo(t *testing.T, dir string) {
	t.Helper()

	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatalf("create repo dir: %v", err)
	}

	mustRunGit(t, dir, "init")
	if err := os.WriteFile(filepath.Join(dir, "README.md"), []byte("demo\n"), 0o644); err != nil {
		t.Fatalf("write README: %v", err)
	}
	mustRunGit(t, dir, "add", "README.md")
	mustRunGit(t, dir, "commit", "-m", "initial commit")
}

func assertSamePath(t *testing.T, got, want string) {
	t.Helper()

	if resolveTestPath(got) != resolveTestPath(want) {
		t.Fatalf("path = %q, want %q", got, want)
	}
}

func resolveTestPath(path string) string {
	if resolved, err := filepath.EvalSymlinks(path); err == nil {
		return resolved
	}

	dir, base := filepath.Dir(path), filepath.Base(path)
	if resolvedDir, err := filepath.EvalSymlinks(dir); err == nil {
		return filepath.Join(resolvedDir, base)
	}

	abs, err := filepath.Abs(path)
	if err != nil {
		return path
	}

	return abs
}

func assertWorktreeListed(t *testing.T, repo, path, branch string) {
	t.Helper()

	canonical, err := filepath.EvalSymlinks(path)
	if err != nil {
		t.Fatalf("resolve worktree path: %v", err)
	}

	list := mustGitOutput(t, repo, "worktree", "list", "--porcelain")
	if !strings.Contains(list, "worktree "+canonical) {
		t.Fatalf("worktree list missing %q:\n%s", canonical, list)
	}
	if !strings.Contains(list, "branch refs/heads/"+branch) {
		t.Fatalf("worktree list missing branch %q:\n%s", branch, list)
	}
}

func mustGitOutput(t *testing.T, dir string, args ...string) string {
	t.Helper()

	cmd := testGitCommand(dir, args...)
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("git %s: %v", strings.Join(args, " "), err)
	}

	return strings.TrimSpace(string(out))
}

func mustRunGit(t *testing.T, dir string, args ...string) {
	t.Helper()

	cmd := testGitCommand(dir, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %s: %v\n%s", strings.Join(args, " "), err, out)
	}
}

func testGitCommand(dir string, args ...string) *exec.Cmd {
	all := append([]string{
		"-c", "init.defaultBranch=main",
		"-c", "user.name=cli-toolbox",
		"-c", "user.email=cli-toolbox@example.com",
		"-c", "commit.gpgsign=false",
	}, args...)
	cmd := exec.Command("git", all...)
	cmd.Dir = dir
	return cmd
}
