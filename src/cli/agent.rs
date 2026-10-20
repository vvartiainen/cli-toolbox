use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::{Error, agent, git};

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct AgentCmd {
    #[command(subcommand)]
    command: AgentCommands,
}

#[derive(Debug, Subcommand)]
enum AgentCommands {
    /// Manage sibling git worktrees for AI agents.
    #[command(
        subcommand_required = true,
        subcommand_value_name = "command",
        after_help = "Worktrees are created next to the current directory as <cwd>-agent-wt, <cwd>-agent-wt-2, and so on."
    )]
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
}

#[derive(Debug, Subcommand)]
pub(super) enum WorktreeCommands {
    /// Create a sibling git worktree and branch for AI agents, initialize it and open it in Herdr.
    #[command(
        after_help = "Creates a new worktree and branch next to the current directory and prints the path.\n\nThe worktree is then initialized: a `worktree-init` recipe in the project's justfile runs if there is one (with CLI_TOOLBOX_WORKTREE_SOURCE set to the source checkout), otherwise the install of the package manager whose lockfile is present (npm ci, pnpm install, yarn install, bun install). If herdr is installed, the worktree opens as a focused Herdr workspace.\n\nShell usage:\n\n  cd \"$(cli-toolbox agent worktree create)\""
    )]
    Create {
        /// Skip the worktree-init recipe or package install.
        #[arg(long)]
        no_init: bool,
        /// Do not open the worktree in Herdr.
        #[arg(long)]
        no_herdr: bool,
    },
    /// List agent worktrees for the current directory.
    List,
    /// Remove an agent worktree and its branch, and close its Herdr workspace.
    #[command(
        after_help = "Removes an agent worktree and its branch.\n\nIf name is omitted and only one agent worktree exists, that one is removed. A Herdr workspace that has the worktree open is closed after the removal succeeds."
    )]
    Cleanup {
        /// Worktree name or path to remove.
        name: Option<String>,
        /// Remove all agent worktrees for the current directory.
        #[arg(long)]
        all: bool,
        /// Do not close the worktree's Herdr workspace.
        #[arg(long)]
        no_herdr: bool,
    },
}

impl AgentCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self.command {
            AgentCommands::Worktree { command } => run_worktree(command, stdout, stderr),
        }
    }
}

pub(super) fn run_worktree(
    command: WorktreeCommands,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    match command {
        WorktreeCommands::Create { no_init, no_herdr } => {
            let options = agent::Options {
                init: !no_init,
                herdr: !no_herdr,
            };
            agent::create(
                &working_dir()?,
                &git::worktrees_root()?,
                options,
                stdout,
                stderr,
            )
        }
        WorktreeCommands::List => git::list_agent_worktrees(&working_dir()?, stdout, stderr),
        WorktreeCommands::Cleanup {
            name,
            all,
            no_herdr,
        } => {
            let options = agent::Options {
                herdr: !no_herdr,
                ..agent::Options::default()
            };
            agent::cleanup(
                &working_dir()?,
                name.as_deref().unwrap_or(""),
                all,
                options,
                stdout,
                stderr,
            )
        }
    }
}

fn working_dir() -> Result<PathBuf, Error> {
    std::env::current_dir().map_err(|err| Error::msg(format!("resolve working directory: {err}")))
}
