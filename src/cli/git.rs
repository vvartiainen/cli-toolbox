use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::{Error, git};

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct GitCmd {
    #[command(subcommand)]
    command: GitCommands,
}

#[derive(Debug, Subcommand)]
enum GitCommands {
    /// Manage sibling git worktrees for AI agents.
    #[command(
        name = "agent-worktree",
        subcommand_required = true,
        subcommand_value_name = "command",
        after_help = "Worktrees are created next to the current directory as <cwd>-agent-wt, <cwd>-agent-wt-2, and so on."
    )]
    AgentWorktree {
        #[command(subcommand)]
        command: AgentWorktreeCommands,
    },
}

#[derive(Debug, Subcommand)]
enum AgentWorktreeCommands {
    /// Create a sibling git worktree and branch for AI agents.
    #[command(
        after_help = "Creates a new worktree and branch next to the current directory and prints the path.\n\nShell usage:\n\n  cd \"$(cli-toolbox git agent-worktree create)\""
    )]
    Create,
    /// List agent worktrees for the current directory.
    List,
    /// Remove an agent worktree and its branch.
    #[command(
        after_help = "Removes an agent worktree and its branch.\n\nIf name is omitted and only one agent worktree exists, that one is removed."
    )]
    Cleanup {
        /// Worktree name or path to remove.
        name: Option<String>,
        /// Remove all agent worktrees for the current directory.
        #[arg(long)]
        all: bool,
    },
}

impl GitCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self.command {
            GitCommands::AgentWorktree { command } => match command {
                AgentWorktreeCommands::Create => {
                    git::create_agent_worktree(&working_dir()?, stdout, stderr)
                }
                AgentWorktreeCommands::List => {
                    git::list_agent_worktrees(&working_dir()?, stdout, stderr)
                }
                AgentWorktreeCommands::Cleanup { name, all } => git::cleanup_agent_worktree(
                    &working_dir()?,
                    name.as_deref().unwrap_or(""),
                    all,
                    stdout,
                    stderr,
                ),
            },
        }
    }
}

fn working_dir() -> Result<PathBuf, Error> {
    std::env::current_dir().map_err(|err| Error::msg(format!("resolve working directory: {err}")))
}
