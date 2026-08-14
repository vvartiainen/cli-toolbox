package git

import (
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

const agentWorktreeSuffix = "-agent-wt"

type AgentWorktree struct {
	Name   string
	Path   string
	Branch string
}

func CreateAgentWorktree(cwd string, stdout, stderr io.Writer) error {
	if err := requireGit(); err != nil {
		return err
	}

	layout, err := resolveCwd(cwd)
	if err != nil {
		return err
	}

	used, err := usedAgentWorktreeNames(cwd, layout)
	if err != nil {
		return err
	}

	name := nextAgentWorktreeName(layout.Base, used)
	path := filepath.Join(layout.Parent, name)

	if err := runGit(cwd, stderr, stderr, "worktree", "add", "-b", name, path); err != nil {
		return fmt.Errorf("create git worktree %q on branch %q: %w", path, name, err)
	}

	_, err = fmt.Fprintln(stdout, path)
	return err
}

func ListAgentWorktrees(cwd string, stdout, stderr io.Writer) error {
	worktrees, err := AgentWorktrees(cwd)
	if err != nil {
		return err
	}

	for _, wt := range worktrees {
		if _, err := fmt.Fprintln(stdout, wt.Path); err != nil {
			return err
		}
	}

	return nil
}

func CleanupAgentWorktree(cwd, name string, all bool, stdout, stderr io.Writer) error {
	if err := requireGit(); err != nil {
		return err
	}

	if all && name != "" {
		return fmt.Errorf("specify either a worktree name or --all, not both")
	}

	worktrees, err := AgentWorktrees(cwd)
	if err != nil {
		return err
	}
	if len(worktrees) == 0 {
		return fmt.Errorf("no agent worktrees found")
	}

	selected := worktrees
	if !all {
		wt, err := selectAgentWorktree(worktrees, name)
		if err != nil {
			return err
		}
		selected = []AgentWorktree{wt}
	}

	for _, wt := range selected {
		if err := removeAgentWorktree(cwd, wt, stderr); err != nil {
			return err
		}
		if _, err := fmt.Fprintln(stdout, wt.Path); err != nil {
			return err
		}
	}

	return nil
}

func AgentWorktrees(cwd string) ([]AgentWorktree, error) {
	if err := requireGit(); err != nil {
		return nil, err
	}

	layout, err := resolveCwd(cwd)
	if err != nil {
		return nil, err
	}

	listed, err := listGitWorktrees(cwd)
	if err != nil {
		return nil, err
	}

	worktrees := make([]AgentWorktree, 0, len(listed))
	for _, wt := range listed {
		name := filepath.Base(wt.Path)
		if !isAgentWorktreeName(layout.Base, name) {
			continue
		}

		worktrees = append(worktrees, AgentWorktree{
			Name:   name,
			Path:   wt.Path,
			Branch: wt.Branch,
		})
	}

	sort.Slice(worktrees, func(i, j int) bool {
		return worktreeNameLess(worktrees[i].Name, worktrees[j].Name)
	})

	return worktrees, nil
}

type cwdLayout struct {
	Abs    string
	Parent string
	Base   string
}

type gitWorktree struct {
	Path   string
	Branch string
}

func resolveCwd(cwd string) (cwdLayout, error) {
	abs, err := filepath.Abs(cwd)
	if err != nil {
		return cwdLayout{}, fmt.Errorf("resolve working directory: %w", err)
	}
	if resolved, err := filepath.EvalSymlinks(abs); err == nil {
		abs = resolved
	}

	return cwdLayout{
		Abs:    abs,
		Parent: filepath.Dir(abs),
		Base:   filepath.Base(abs),
	}, nil
}

func nextAgentWorktreeName(base string, used map[string]struct{}) string {
	for n := 1; ; n++ {
		name := agentWorktreeName(base, n)
		if _, exists := used[name]; !exists {
			return name
		}
	}
}

func agentWorktreeName(base string, n int) string {
	if n <= 1 {
		return base + agentWorktreeSuffix
	}

	return fmt.Sprintf("%s%s-%d", base, agentWorktreeSuffix, n)
}

func isAgentWorktreeName(base, name string) bool {
	prefix := base + agentWorktreeSuffix
	if name == prefix {
		return true
	}

	rest, ok := strings.CutPrefix(name, prefix+"-")
	if !ok || rest == "" {
		return false
	}

	n, err := strconv.Atoi(rest)
	return err == nil && n >= 1 && rest[0] != '0'
}

func worktreeNameLess(a, b string) bool {
	ai, aok := agentWorktreeIndex(a)
	bi, bok := agentWorktreeIndex(b)
	if aok && bok {
		return ai < bi
	}

	return a < b
}

func agentWorktreeIndex(name string) (int, bool) {
	suffix := agentWorktreeSuffix
	idx := strings.LastIndex(name, suffix)
	if idx < 0 {
		return 0, false
	}

	rest := name[idx+len(suffix):]
	if rest == "" {
		return 1, true
	}

	n, err := strconv.Atoi(strings.TrimPrefix(rest, "-"))
	if err != nil || n < 1 {
		return 0, false
	}

	return n, true
}

func usedAgentWorktreeNames(cwd string, layout cwdLayout) (map[string]struct{}, error) {
	used := map[string]struct{}{}

	entries, err := os.ReadDir(layout.Parent)
	if err != nil {
		return nil, fmt.Errorf("read %q: %w", layout.Parent, err)
	}
	for _, entry := range entries {
		if isAgentWorktreeName(layout.Base, entry.Name()) {
			used[entry.Name()] = struct{}{}
		}
	}

	worktrees, err := listGitWorktrees(cwd)
	if err != nil {
		return nil, err
	}
	for _, wt := range worktrees {
		name := filepath.Base(wt.Path)
		if isAgentWorktreeName(layout.Base, name) {
			used[name] = struct{}{}
		}
		if wt.Branch != "" && isAgentWorktreeName(layout.Base, wt.Branch) {
			used[wt.Branch] = struct{}{}
		}
	}

	branches, err := gitOutput(cwd, "for-each-ref", "--format=%(refname:short)", "refs/heads")
	if err != nil {
		return nil, err
	}
	for _, branch := range strings.Split(branches, "\n") {
		branch = strings.TrimSpace(branch)
		if isAgentWorktreeName(layout.Base, branch) {
			used[branch] = struct{}{}
		}
	}

	return used, nil
}

func listGitWorktrees(cwd string) ([]gitWorktree, error) {
	output, err := gitOutput(cwd, "worktree", "list", "--porcelain")
	if err != nil {
		return nil, err
	}

	return parseWorktreePorcelain(output), nil
}

func parseWorktreePorcelain(output string) []gitWorktree {
	var worktrees []gitWorktree
	var current gitWorktree

	flush := func() {
		if current.Path == "" {
			return
		}
		worktrees = append(worktrees, current)
		current = gitWorktree{}
	}

	for line := range strings.SplitSeq(output, "\n") {
		if line == "" {
			flush()
			continue
		}

		key, value, _ := strings.Cut(line, " ")
		switch key {
		case "worktree":
			current.Path = value
		case "branch":
			current.Branch = strings.TrimPrefix(value, "refs/heads/")
		}
	}
	flush()

	return worktrees
}

func selectAgentWorktree(worktrees []AgentWorktree, name string) (AgentWorktree, error) {
	if name == "" {
		if len(worktrees) == 1 {
			return worktrees[0], nil
		}

		return AgentWorktree{}, fmt.Errorf("multiple agent worktrees found; specify a name or --all")
	}

	for _, wt := range worktrees {
		if worktreeMatches(wt, name) {
			return wt, nil
		}
	}

	return AgentWorktree{}, fmt.Errorf("agent worktree %q not found", name)
}

func worktreeMatches(wt AgentWorktree, query string) bool {
	if query == wt.Name || query == wt.Path {
		return true
	}

	abs, err := filepath.Abs(query)
	if err != nil {
		return false
	}
	if abs == wt.Path {
		return true
	}

	resolved, err := filepath.EvalSymlinks(abs)
	return err == nil && resolved == wt.Path
}

func removeAgentWorktree(cwd string, wt AgentWorktree, stderr io.Writer) error {
	if err := runGit(cwd, stderr, stderr, "worktree", "remove", wt.Path); err != nil {
		return fmt.Errorf("remove git worktree %q: %w", wt.Path, err)
	}

	if wt.Branch == "" {
		return nil
	}

	if err := runGit(cwd, stderr, stderr, "branch", "-D", "--", wt.Branch); err != nil {
		return fmt.Errorf("delete git branch %q: %w", wt.Branch, err)
	}

	return nil
}

func requireGit() error {
	if _, err := exec.LookPath("git"); err != nil {
		return fmt.Errorf("git is not available on PATH")
	}

	return nil
}

func runGit(cwd string, stdout, stderr io.Writer, args ...string) error {
	cmd := exec.Command("git", args...)
	cmd.Dir = cwd
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	return cmd.Run()
}

func gitOutput(cwd string, args ...string) (string, error) {
	cmd := exec.Command("git", args...)
	cmd.Dir = cwd
	out, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("git %s: %w\n%s", strings.Join(args, " "), err, strings.TrimSpace(string(out)))
	}

	return strings.TrimSpace(string(out)), nil
}
