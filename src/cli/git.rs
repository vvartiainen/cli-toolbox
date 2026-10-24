use std::io::Write;

use clap::{Args, Subcommand};

use super::agent::{LaunchArgs, working_dir};
use crate::{Error, agent, git};

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct GitCmd {
    #[command(subcommand)]
    command: GitCommands,
}

#[derive(Debug, Subcommand)]
enum GitCommands {
    /// Manage git worktrees of the current repository.
    #[command(
        subcommand_required = true,
        subcommand_value_name = "command",
        after_help = "Worktrees are created in ~/.worktrees/<repo>/<name> (or $CLI_TOOLBOX_GIT_WORKTREES_DIR/<repo>/<name>). Sandbox project worktrees (`agent project add`) and review checkouts are not listed here."
    )]
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
}

#[derive(Debug, Subcommand)]
enum WorktreeCommands {
    /// Create a named git worktree and branch, initialize it and open it in Herdr.
    #[command(
        after_help = "Creates ~/.worktrees/<repo>/<dir> on the branch NAME and prints the path. NAME is the branch as typed (feature/login); the folder is NAME with `/` replaced by `-` (feature-login). An existing local branch is checked out, otherwise it is created.\n\nThe worktree is then initialized: a `worktree-init` recipe in the project's justfile runs if there is one (with CLI_TOOLBOX_WORKTREE_SOURCE set to the source checkout), otherwise the install of the package manager whose lockfile is present (npm ci, pnpm install, yarn install, bun install). If herdr is installed, the worktree opens as a focused Herdr workspace. With --agent or --prompt, that agent starts there and gets the prompt.\n\nShell usage:\n\n  cd \"$(cli-toolbox git worktree create feature/login)\"\n  cli-toolbox git worktree create fix-login --agent codex --prompt \"Fix the flaky login test\""
    )]
    Create {
        /// Branch name; the folder name is derived from it.
        name: String,
        /// Skip the worktree-init recipe or package install.
        #[arg(long)]
        no_init: bool,
        /// Do not open the worktree in Herdr.
        #[arg(long, conflicts_with_all = ["agent", "prompt"])]
        no_herdr: bool,
        #[command(flatten)]
        launch: LaunchArgs,
    },
    /// List the git worktrees of the current repository.
    List,
    /// Remove a git worktree and its branch, and close its Herdr workspace.
    #[command(
        after_help = "Removes a git worktree and its branch.\n\nIf NAME is omitted and only one worktree exists, that one is removed. NAME is the folder, the branch or the path. A Herdr workspace that has the worktree open is closed after the removal succeeds."
    )]
    Remove {
        /// Worktree folder, branch or path to remove.
        name: Option<String>,
        /// Remove all git worktrees of the current repository.
        #[arg(long)]
        all: bool,
        /// Keep the worktree's branch.
        #[arg(long)]
        keep_branch: bool,
        /// Do not close the worktree's Herdr workspace.
        #[arg(long)]
        no_herdr: bool,
    },
}

impl GitCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self.command {
            GitCommands::Worktree { command } => run_worktree(command, stdout, stderr),
        }
    }
}

fn run_worktree(
    command: WorktreeCommands,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let cwd = working_dir()?;
    match command {
        WorktreeCommands::Create {
            name,
            no_init,
            no_herdr,
            launch,
        } => {
            let options = agent::Options {
                init: !no_init,
                herdr: !no_herdr,
            };
            agent::create(
                &cwd,
                &git::git_worktrees_root()?,
                &name,
                options,
                launch.resolve()?.as_ref(),
                stdout,
                stderr,
            )
        }
        WorktreeCommands::List => {
            for worktree in agent::git_worktrees(&cwd)? {
                writeln!(stdout, "{}", worktree.path.display())?;
            }
            Ok(())
        }
        WorktreeCommands::Remove {
            name,
            all,
            keep_branch,
            no_herdr,
        } => {
            let options = agent::Options {
                herdr: !no_herdr,
                ..agent::Options::default()
            };
            agent::remove(
                &cwd,
                agent::git_worktrees(&cwd)?,
                name.as_deref().unwrap_or(""),
                all,
                keep_branch,
                options,
                stdout,
                stderr,
            )
        }
    }
}
