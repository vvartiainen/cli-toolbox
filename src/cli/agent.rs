use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::agent::vm::{self, HomeFrom, VmOptions};
use crate::sandbox::init::{self, InitOptions, SshKey};
use crate::{Error, agent, git};

/// The default agent kind for `--agent`.
const AGENT_ENV: &str = "CLI_TOOLBOX_AGENT";

const VM_HELP: &str = "Each sandboxed agent worktree runs its own agent-sandbox microVM from the project's sandbox flake (sandbox/flake.nix). The VM is added to herdr as a machine named after the worktree, and the agent runs in a herdr workspace inside the guest.\n\nWorkflow:\n\n  cli-toolbox agent sandbox init --ai   # once per project\n  cli-toolbox agent start --agent claude\n  cli-toolbox agent sync                # the agent's commits land on the worktree branch\n  cli-toolbox agent done                # stop the VM, remove the worktree, keep the branch";

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct AgentCmd {
    #[command(subcommand)]
    command: AgentCommands,
}

#[derive(Debug, Subcommand)]
enum AgentCommands {
    /// Create an agent worktree, boot its sandbox VM and open the agent in herdr.
    #[command(after_help = VM_HELP)]
    Start {
        #[command(flatten)]
        vm: VmArgs,
        #[command(flatten)]
        boot: BootArgs,
    },
    /// Boot the VM of an existing agent worktree again.
    Up {
        /// Worktree name or path; may be omitted when there is only one.
        name: Option<String>,
        #[command(flatten)]
        vm: VmArgs,
        #[command(flatten)]
        boot: BootArgs,
    },
    /// Stop the VM of an agent worktree, keeping the worktree.
    Stop {
        /// Worktree name or path; may be omitted when there is only one.
        name: Option<String>,
        #[command(flatten)]
        vm: VmArgs,
    },
    /// Bring the agent's commits to the worktree branch on the host.
    #[command(
        after_help = "Fetches the guest's commits into sandbox/* and fast-forwards the worktree branch, which then exists in the main repository for review and merge. Files are already shared live; only git history moves."
    )]
    Sync {
        /// Worktree name or path; may be omitted when there is only one.
        name: Option<String>,
        #[command(flatten)]
        vm: VmArgs,
    },
    /// List agent worktrees and their VMs.
    List,
    /// Sync, stop the VM, remove the herdr machine and the worktree; keep the branch.
    #[command(
        after_help = "Refuses when guest commits could not be synced, when the VM is stopped but has a home image, or when the worktree has uncommitted changes. --force removes anyway."
    )]
    Done {
        /// Worktree name or path; may be omitted when there is only one.
        name: Option<String>,
        /// Remove even if commits or changes would be lost.
        #[arg(long)]
        force: bool,
        /// Keep the herdr machine.
        #[arg(long)]
        no_herdr: bool,
        #[command(flatten)]
        vm: VmArgs,
    },
    /// Set up the project's agent-sandbox flake.
    #[command(subcommand_required = true, subcommand_value_name = "command")]
    Sandbox {
        #[command(subcommand)]
        command: SandboxCommands,
    },
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

#[derive(Debug, Args)]
struct VmArgs {
    /// Sandbox flake directory (default: sandbox/flake.nix in the worktree or main checkout).
    #[arg(long, value_name = "DIR")]
    flake: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BootArgs {
    /// Start from a copy of a home image ("main" for the main checkout's) instead of a fresh home.
    #[arg(long, value_name = "main|PATH")]
    home_from: Option<String>,
    /// Agent kind herdr starts in the guest, e.g. claude or codex (default: $CLI_TOOLBOX_AGENT).
    #[arg(long, value_name = "KIND")]
    agent: Option<String>,
    /// Do not add the VM to herdr.
    #[arg(long)]
    no_herdr: bool,
}

#[derive(Debug, Subcommand)]
enum SandboxCommands {
    /// Create sandbox/flake.nix from an agent-sandbox template.
    #[command(
        after_help = "Creates the flake from `nix flake init -t <agent-sandbox>#<template>`, points its agent-sandbox input at the same flake, names the VM after the project, authorizes your SSH public key, adds .agent-sandbox/ to .gitignore and marks the flake for git (Nix only sees tracked files).\n\nWith --ai, a coding agent then tailors the toolchain to the repository following agent-sandbox's SETUP-AGENT.md.\n\nThe agent-sandbox flake defaults to $CLI_TOOLBOX_AGENT_SANDBOX_FLAKE, then the upstream repository."
    )]
    Init {
        /// Template: nodejs, rust or go.
        #[arg(long, default_value = "nodejs")]
        template: String,
        /// Directory for flake.nix, relative to the main checkout.
        #[arg(long, default_value = "sandbox")]
        dir: PathBuf,
        /// agent-sandbox flake reference to use.
        #[arg(long, value_name = "FLAKE")]
        from: Option<String>,
        /// SSH public key to authorize (default: ~/.ssh/*.pub, then the SSH agent).
        #[arg(long, value_name = "PATH", conflicts_with = "no_ssh_key")]
        ssh_key: Option<PathBuf>,
        /// Keep the template's placeholder key.
        #[arg(long)]
        no_ssh_key: bool,
        /// Let a coding agent tailor the flake: optionally the agent kind (default: $CLI_TOOLBOX_AGENT or claude).
        #[arg(long, value_name = "KIND", num_args = 0..=1, default_missing_value = "")]
        ai: Option<String>,
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
            AgentCommands::Start { vm: args, boot } => vm::start(
                &working_dir()?,
                &git::worktrees_root()?,
                &vm_options(args, Some(boot), false)?,
                stdout,
                stderr,
            ),
            AgentCommands::Up {
                name,
                vm: args,
                boot,
            } => {
                let options = vm_options(args, Some(boot), false)?;
                let steps = vm::up(&working_dir()?, &name.unwrap_or_default(), &options, stderr)?;
                finish(&steps, stderr)
            }
            AgentCommands::Stop { name, vm: args } => {
                let options = vm_options(args, None, false)?;
                let steps = vm::stop(&working_dir()?, &name.unwrap_or_default(), &options)?;
                finish(&steps, stderr)
            }
            AgentCommands::Sync { name, vm: args } => vm::sync(
                &working_dir()?,
                &name.unwrap_or_default(),
                &vm_options(args, None, false)?,
                stdout,
                stderr,
            ),
            AgentCommands::List => vm::list(&working_dir()?, stdout),
            AgentCommands::Done {
                name,
                force,
                no_herdr,
                vm: args,
            } => {
                let mut options = vm_options(args, None, force)?;
                options.herdr = !no_herdr;
                vm::done(
                    &working_dir()?,
                    &name.unwrap_or_default(),
                    &options,
                    stdout,
                    stderr,
                )
            }
            AgentCommands::Sandbox {
                command:
                    SandboxCommands::Init {
                        template,
                        dir,
                        from,
                        ssh_key,
                        no_ssh_key,
                        ai,
                    },
            } => {
                let options = InitOptions {
                    template,
                    dir,
                    source: from,
                    ssh_key: match (ssh_key, no_ssh_key) {
                        (Some(path), _) => SshKey::File(path),
                        (None, true) => SshKey::None,
                        (None, false) => SshKey::Auto,
                    },
                    ai: ai.map(|kind| {
                        if kind.is_empty() {
                            default_agent().unwrap_or_else(|| "claude".into())
                        } else {
                            kind
                        }
                    }),
                };
                let flake = init::init(&working_dir()?, &options, stderr)?;
                writeln!(stdout, "{}", flake.display())?;
                Ok(())
            }
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

fn vm_options(args: VmArgs, boot: Option<BootArgs>, force: bool) -> Result<VmOptions, Error> {
    let mut options = VmOptions {
        flake: args.flake,
        herdr: true,
        force,
        ..VmOptions::default()
    };
    if let Some(boot) = boot {
        options.home_from = boot.home_from.as_deref().map(home_from);
        options.agent = boot.agent.or_else(default_agent);
        options.herdr = !boot.no_herdr;
    }
    Ok(options)
}

fn home_from(value: &str) -> HomeFrom {
    if value == "main" {
        HomeFrom::Main
    } else {
        HomeFrom::Image(Path::new(value).to_path_buf())
    }
}

fn default_agent() -> Option<String> {
    std::env::var(AGENT_ENV)
        .ok()
        .filter(|kind| !kind.is_empty())
}

fn finish(steps: &[agent::Step], stderr: &mut dyn Write) -> Result<(), Error> {
    for step in steps {
        writeln!(stderr, "{step}")?;
    }
    if steps.iter().any(agent::Step::failed) {
        Err(Error::msg("a step failed"))
    } else {
        Ok(())
    }
}

fn working_dir() -> Result<PathBuf, Error> {
    std::env::current_dir().map_err(|err| Error::msg(format!("resolve working directory: {err}")))
}
