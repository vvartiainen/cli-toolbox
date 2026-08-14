package git

import (
	"fmt"
	"io"
	"os/exec"
	"path/filepath"
)

func CreateAgentWorktree(cwd string, stdout, stderr io.Writer) error {
	path, branch, err := agentWorktreeTarget(cwd)
	if err != nil {
		return err
	}

	if _, err := exec.LookPath("git"); err != nil {
		return fmt.Errorf("git is not available on PATH")
	}

	cmd := exec.Command("git", "worktree", "add", "-b", branch, path)
	cmd.Dir = cwd
	cmd.Stdout = stderr
	cmd.Stderr = stderr
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("create git worktree %q on branch %q: %w", path, branch, err)
	}

	_, err = fmt.Fprintln(stdout, path)
	return err
}

func agentWorktreeTarget(cwd string) (string, string, error) {
	abs, err := filepath.Abs(cwd)
	if err != nil {
		return "", "", fmt.Errorf("resolve working directory: %w", err)
	}

	branch := filepath.Base(abs) + "-agent-wt"
	path := filepath.Join(filepath.Dir(abs), branch)
	return path, branch, nil
}
