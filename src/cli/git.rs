use std::io::Write;

use clap::{Args, Subcommand};

use super::agent::{WorktreeCommands, run_worktree};
use crate::Error;

/// Kept as a hidden alias of `agent worktree` for existing scripts.
#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct GitCmd {
    #[command(subcommand)]
    command: GitCommands,
}

#[derive(Debug, Subcommand)]
enum GitCommands {
    /// Manage sibling git worktrees for AI agents (alias of `agent worktree`).
    #[command(
        name = "agent-worktree",
        subcommand_required = true,
        subcommand_value_name = "command",
        after_help = "Deprecated alias of `cli-toolbox agent worktree`."
    )]
    AgentWorktree {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
}

impl GitCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self.command {
            GitCommands::AgentWorktree { command } => run_worktree(command, stdout, stderr),
        }
    }
}
