package git

import (
	"bytes"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func TestAgentWorktreeTargetUsesParentAndSuffix(t *testing.T) {
	dir := filepath.Join(t.TempDir(), "cli-toolbox")
	if err := os.Mkdir(dir, 0o755); err != nil {
		t.Fatalf("create cwd: %v", err)
	}

	path, branch, err := agentWorktreeTarget(dir)
	if err != nil {
		t.Fatalf("agentWorktreeTarget returned error: %v", err)
	}

	wantPath := filepath.Join(filepath.Dir(dir), "cli-toolbox-agent-wt")
	if path != wantPath {
		t.Fatalf("path = %q, want %q", path, wantPath)
	}
	if branch != "cli-toolbox-agent-wt" {
		t.Fatalf("branch = %q, want %q", branch, "cli-toolbox-agent-wt")
	}
}

func TestCreateAgentWorktreeCreatesSiblingWorktreeAndBranch(t *testing.T) {
	requireGit(t)

	root := t.TempDir()
	repo := filepath.Join(root, "demo")
	initGitRepo(t, repo)

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	if err := CreateAgentWorktree(repo, &stdout, &stderr); err != nil {
		t.Fatalf("CreateAgentWorktree returned error: %v\nstderr: %s", err, stderr.String())
	}

	wantPath := filepath.Join(root, "demo-agent-wt")
	if got := strings.TrimSpace(stdout.String()); got != wantPath {
		t.Fatalf("stdout = %q, want %q", got, wantPath)
	}

	if _, err := os.Stat(wantPath); err != nil {
		t.Fatalf("worktree path missing: %v", err)
	}

	if got, want := gitOutput(t, wantPath, "rev-parse", "--abbrev-ref", "HEAD"), "demo-agent-wt"; got != want {
		t.Fatalf("worktree branch = %q, want %q", got, want)
	}

	canonical, err := filepath.EvalSymlinks(wantPath)
	if err != nil {
		t.Fatalf("resolve worktree path: %v", err)
	}

	list := gitOutput(t, repo, "worktree", "list", "--porcelain")
	if !strings.Contains(list, "worktree "+canonical) {
		t.Fatalf("worktree list missing %q:\n%s", canonical, list)
	}
	if !strings.Contains(list, "branch refs/heads/demo-agent-wt") {
		t.Fatalf("worktree list missing branch:\n%s", list)
	}
}

func requireGit(t *testing.T) {
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

	runGit(t, dir, "init")
	if err := os.WriteFile(filepath.Join(dir, "README.md"), []byte("demo\n"), 0o644); err != nil {
		t.Fatalf("write README: %v", err)
	}
	runGit(t, dir, "add", "README.md")
	runGit(t, dir, "commit", "-m", "initial commit")
}

func gitOutput(t *testing.T, dir string, args ...string) string {
	t.Helper()

	cmd := gitCommand(dir, args...)
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("git %s: %v", strings.Join(args, " "), err)
	}

	return strings.TrimSpace(string(out))
}

func runGit(t *testing.T, dir string, args ...string) {
	t.Helper()

	cmd := gitCommand(dir, args...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %s: %v\n%s", strings.Join(args, " "), err, out)
	}
}

func gitCommand(dir string, args ...string) *exec.Cmd {
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
