use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::agent::vm::{self, VmOptions};
use crate::sandbox::project::{self, CreateOptions, Project, SshKey};
use crate::{Error, agent, git};

/// The default agent kind for `--agent`.
const AGENT_ENV: &str = "CLI_TOOLBOX_AGENT";

const PROJECT_HELP: &str = "A sandbox project is a directory in ~/.agent-sandbox/projects/ (or $CLI_TOOLBOX_SANDBOX_DIR/projects/) holding an agent-sandbox flake, out of the agents' reach. Its one microVM mounts every agent worktree of the project's repository at /workspace/<worktree>; cli-toolbox writes that list to worktrees.nix next to the flake. The VM is added to herdr as the machine agent-<project>, with a guest workspace per worktree.\n\nWorkflow:\n\n  cli-toolbox agent project create --ai   # in the repository, once\n  cli-toolbox agent project start --agent claude\n  cli-toolbox agent project add --restart # another worktree for another agent\n  cli-toolbox agent project sync          # the agents' commits land on the worktree branches\n  cli-toolbox agent project done <worktree>\n\nPROJECT defaults to the project of the current repository, or the only project.";

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct AgentCmd {
    #[command(subcommand)]
    command: AgentCommands,
}

#[derive(Debug, Subcommand)]
enum AgentCommands {
    /// Sandbox projects: one agent-sandbox VM per project, with its worktrees.
    #[command(
        subcommand_required = true,
        subcommand_value_name = "command",
        after_help = PROJECT_HELP
    )]
    Project {
        #[command(subcommand)]
        command: ProjectCommands,
    },
    /// Every running agent-sandbox VM, also those whose project is gone.
    #[command(
        subcommand_required = true,
        subcommand_value_name = "command",
        after_help = "Works from the runner.json files in ${AGENT_SANDBOX_RUNTIME_ROOT:-${XDG_STATE_HOME:-~/.local/state}/agent-sandbox}/vms, so it also reaches VMs whose project or flake directory was removed. Use `agent project stop` for the VM of an existing project."
    )]
    Vm {
        #[command(subcommand)]
        command: VmCommands,
    },
    /// Manage git worktrees for AI agents.
    #[command(
        subcommand_required = true,
        subcommand_value_name = "command",
        after_help = "Worktrees are created in ~/.agent-sandbox/worktrees/<repo>/ as <repo>-agent-wt, <repo>-agent-wt-2, and so on."
    )]
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommands,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectCommands {
    /// List sandbox projects, their VMs and worktrees.
    List,
    /// Create a sandbox project for the current repository and its first worktree.
    #[command(
        after_help = "Creates <sandbox dir>/projects/<name>/flake.nix from `nix flake init -t <agent-sandbox>#<template>`, points its agent-sandbox input at the same flake, keeps the guest home in <sandbox dir>/state/<name>, picks a free SSH port (2300-2399), authorizes your SSH public key and mounts the generated worktrees.nix. It adds a cli-toolbox defaults module: Claude Code and Codex with their credentials from agent-proxy on the host, and a Backlog.md backlog in $CLI_TOOLBOX_BACKLOG_ROOT/<name> (default: the Obsidian Agents folder in iCloud, when it exists). Then creates the repository's first agent worktree and runs `agent project setup`.\n\nWith --ai, a coding agent then tailors the toolchain to the repository following agent-sandbox's docs/setup-agent.md.\n\nThe agent-sandbox flake defaults to $CLI_TOOLBOX_AGENT_SANDBOX_FLAKE, then the upstream repository."
    )]
    Create {
        /// Project name (default: the repository's directory name).
        #[arg(long)]
        name: Option<String>,
        /// Template: nodejs, rust or go (default: from the repository's files).
        #[arg(long)]
        template: Option<String>,
        /// agent-sandbox flake reference to use.
        #[arg(long, value_name = "FLAKE")]
        from: Option<String>,
        /// SSH public key to authorize (default: the ssh config's IdentityFile for the VM, ~/.ssh/*.pub, then the SSH agent).
        #[arg(long, value_name = "PATH", conflicts_with = "no_ssh_key")]
        ssh_key: Option<PathBuf>,
        /// Keep the template's placeholder key.
        #[arg(long)]
        no_ssh_key: bool,
        /// Let a coding agent tailor the flake: optionally the agent kind (default: $CLI_TOOLBOX_AGENT or claude).
        #[arg(long, value_name = "KIND", num_args = 0..=1, default_missing_value = "")]
        ai: Option<String>,
        /// Do not take agent credentials from agent-proxy.
        #[arg(long)]
        no_proxy: bool,
        /// Do not give the project a backlog.
        #[arg(long)]
        no_backlog: bool,
    },
    /// Set up the host side of the project's proxy credentials and backlog.
    #[command(
        after_help = "When flake.nix takes its credentials from agent-proxy (caCert = ./agent-proxy-ca.pem), runs agent-sandbox's scripts/agent-proxy-setup.sh for the services claude-code and codex: it stores missing secrets (prompting for them), creates the client named after the project with its token in the login keychain, and writes agent-proxy-ca.pem next to the flake. agent-proxy must be running, and on PATH or in $AGENT_PROXY_FLAKE.\n\nWhen flake.nix sets vm.backlogDir, creates that folder and runs `backlog init` in it unless it already has a backlog.\n\nSafe to re-run; `agent project create` runs it."
    )]
    Setup { project: Option<String> },
    /// Start the project's VM and open its worktrees in herdr.
    Start {
        project: Option<String>,
        #[command(flatten)]
        boot: BootArgs,
    },
    /// Stop the project's VM, keeping its worktrees and guest home.
    Stop { project: Option<String> },
    /// Restart the project's VM, so it mounts the current worktrees.
    Restart {
        project: Option<String>,
        #[command(flatten)]
        boot: BootArgs,
    },
    /// Create another agent worktree for the project; prints its path.
    Add {
        project: Option<String>,
        /// Restart a running VM right away to mount it.
        #[arg(long)]
        restart: bool,
        #[command(flatten)]
        boot: BootArgs,
    },
    /// Bring the agents' commits to the worktree branches on the host.
    #[command(
        after_help = "Fetches the guest's commits into sandbox/* and fast-forwards each worktree branch, which then exists in the main repository for review and merge. Files are already shared live; only git history moves."
    )]
    Sync {
        project: Option<String>,
        /// Only this worktree.
        #[arg(long)]
        worktree: Option<String>,
    },
    /// Sync a worktree, then remove it and keep its branch.
    #[command(
        after_help = "Refuses when guest commits could not be synced, when the VM is stopped but has a guest home, or when the worktree has uncommitted changes. --force removes anyway. The VM stops when its last worktree is done."
    )]
    Done {
        /// Worktree name or path; may be omitted when the project has only one.
        worktree: Option<String>,
        #[arg(long)]
        project: Option<String>,
        /// Remove even if commits or changes would be lost.
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum VmCommands {
    /// List running VMs: host alias, runner PID, SSH port, start time and start directory.
    List,
    /// Stop a running VM and remove its herdr machine.
    #[command(
        after_help = "Sends SIGTERM to the VM's runner, which shuts the VM down; after $AGENT_SANDBOX_STOP_TIMEOUT seconds (default 30) the runner and its VM process are killed. Without NAME, stops the only running VM or picks one."
    )]
    Stop {
        /// Host alias (agent-<project>) or start directory name.
        #[arg(conflicts_with = "all")]
        name: Option<String>,
        /// Stop every running VM.
        #[arg(long)]
        all: bool,
    },
    /// Remove the runtime directories of stopped VMs whose start directory is gone.
    Prune,
}

#[derive(Debug, Args)]
struct BootArgs {
    /// Agent kind herdr starts in each guest workspace, e.g. claude or codex (default: $CLI_TOOLBOX_AGENT).
    #[arg(long, value_name = "KIND")]
    agent: Option<String>,
    /// Do not add the VM to herdr.
    #[arg(long)]
    no_herdr: bool,
}

#[derive(Debug, Subcommand)]
pub(super) enum WorktreeCommands {
    /// Create a git worktree and branch for AI agents, initialize it and open it in Herdr.
    #[command(
        after_help = "Creates a new worktree and branch and prints the path.\n\nThe worktree is then initialized: a `worktree-init` recipe in the project's justfile runs if there is one (with CLI_TOOLBOX_WORKTREE_SOURCE set to the source checkout), otherwise the install of the package manager whose lockfile is present (npm ci, pnpm install, yarn install, bun install). If herdr is installed, the worktree opens as a focused Herdr workspace.\n\nShell usage:\n\n  cd \"$(cli-toolbox agent worktree create)\""
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
            AgentCommands::Project { command } => run_project(command, stdout, stderr),
            AgentCommands::Vm { command } => run_vm(command, stdout, stderr),
        }
    }
}

fn run_project(
    command: ProjectCommands,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let root = project::sandbox_dir()?;
    match command {
        ProjectCommands::List => vm::list(&root, stdout),
        ProjectCommands::Create {
            name,
            template,
            from,
            ssh_key,
            no_ssh_key,
            ai,
            no_proxy,
            no_backlog,
        } => {
            let options = CreateOptions {
                name,
                repo: git::main_checkout(&working_dir()?)?,
                template,
                source: from,
                ssh_key: match (ssh_key, no_ssh_key) {
                    (Some(path), _) => SshKey::File(path),
                    (None, true) => SshKey::None,
                    (None, false) => SshKey::Auto,
                },
                proxy: !no_proxy,
                backlog: !no_backlog,
            };
            let ai = ai.map(|kind| {
                if kind.is_empty() {
                    default_agent().unwrap_or_else(|| "claude".into())
                } else {
                    kind
                }
            });
            let (project, created) = vm::create(
                &root,
                &git::worktrees_root()?,
                &options,
                ai.as_deref(),
                stderr,
            )?;
            finish(&created.steps, stderr)?;
            writeln!(stdout, "{}", project.dir.display())?;
            Ok(())
        }
        ProjectCommands::Setup { project } => finish(&vm::setup(&resolve(&root, project)?), stderr),
        ProjectCommands::Start { project, boot } => {
            let steps = vm::start(&resolve(&root, project)?, &boot_options(boot), stderr)?;
            finish(&steps, stderr)
        }
        ProjectCommands::Stop { project } => finish(&vm::stop(&resolve(&root, project)?)?, stderr),
        ProjectCommands::Restart { project, boot } => {
            let steps = vm::restart(&resolve(&root, project)?, &boot_options(boot), stderr)?;
            finish(&steps, stderr)
        }
        ProjectCommands::Add {
            project,
            restart,
            boot,
        } => {
            let created = vm::add_worktree(
                &resolve(&root, project)?,
                &git::worktrees_root()?,
                restart,
                &boot_options(boot),
                stderr,
            )?;
            writeln!(stdout, "{}", created.worktree.path.display())?;
            finish(&created.steps, stderr)
        }
        ProjectCommands::Sync { project, worktree } => {
            let project = resolve(&root, project)?;
            let worktree = worktree
                .map(|name| vm::select_worktree(&project, &name))
                .transpose()?;
            let synced = vm::sync(&project, worktree.as_ref())?;
            vm::write_synced(&synced, stdout)
        }
        ProjectCommands::Done {
            worktree,
            project,
            force,
        } => {
            let project = resolve(&root, project)?;
            let worktree = vm::select_worktree(&project, &worktree.unwrap_or_default())?;
            let options = VmOptions {
                force,
                ..VmOptions::default()
            };
            let steps = vm::done(&project, &worktree, &options, stderr)?;
            writeln!(stdout, "{}", worktree.branch)?;
            finish(&steps, stderr)
        }
    }
}

fn run_vm(
    command: VmCommands,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    match command {
        VmCommands::List => vm::vm_list(stdout),
        VmCommands::Stop { name, all } => {
            let steps: Vec<_> = vm::select_vms(name.as_deref(), all)?
                .iter()
                .flat_map(vm::vm_stop)
                .collect();
            finish(&steps, stderr)
        }
        VmCommands::Prune => finish(&vm::vm_prune(&project::sandbox_dir()?)?, stderr),
    }
}

/// The named project, else the current repository's, else the only one.
fn resolve(root: &Path, name: Option<String>) -> Result<Project, Error> {
    let repo = working_dir()
        .ok()
        .and_then(|cwd| git::main_checkout(&cwd).ok());
    project::resolve(root, name.as_deref(), repo.as_deref())
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

fn boot_options(boot: BootArgs) -> VmOptions {
    VmOptions {
        agent: boot.agent.or_else(default_agent),
        herdr: !boot.no_herdr,
        force: false,
    }
}

pub(crate) fn default_agent() -> Option<String> {
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
