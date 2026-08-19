package cli

import (
	"os"

	toolboxgit "cli-toolbox/internal/git"

	kongcompletion "github.com/jotaen/kong-completion"
	"github.com/posener/complete"
)

var worktreeNamesPredictor = kongcompletion.WithPredictor("worktree", complete.PredictFunc(predictWorktreeNames))

func predictWorktreeNames(complete.Args) []string {
	cwd, err := os.Getwd()
	if err != nil {
		return nil
	}

	worktrees, err := toolboxgit.AgentWorktrees(cwd)
	if err != nil {
		return nil
	}

	names := make([]string, 0, len(worktrees)*2)
	for _, wt := range worktrees {
		names = append(names, wt.Name, wt.Path)
	}

	return names
}
