use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::agent::Launch;
use crate::agent::vm::{self, VmOptions};
use crate::sandbox::project::{self, CreateOptions, Project, SshKey};
use crate::{Error, agent, git};

/// The default agent kind for `--agent`.
const AGENT_ENV: &str = "CLI_TOOLBOX_AGENT";

const PROJECT_HELP: &str = "A sandbox project is a directory in ~/.agent-sandbox/projects/ (or $CLI_TOOLBOX_SANDBOX_DIR/projects/) holding an agent-sandbox flake, out of the agents' reach. Its one microVM mounts every agent worktree of the project's repository at /workspace/<worktree>; cli-toolbox writes that list to worktrees.nix next to the flake. The VM is added to herdr as the machine agent-<project>, with a guest workspace per worktree.\n\nWorkflow:\n\n  cli-toolbox agent project create --ai   # in the repository, once\n  cli-toolbox agent project start --agent claude\n  cli-toolbox agent project add --restart # another worktree for another agent\n  cli-toolbox agent project sync          # publish agent review snapshot branches\n  cli-toolbox agent project review        # open committed work in lazygit\n  cli-toolbox agent project done <worktree>\n\nPROJECT defaults to the project of the current repository, or the only project.";

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
        #[command(flatten)]
        launch: LaunchArgs,
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
    Stop {
        #[arg(conflicts_with = "all")]
        project: Option<String>,
        /// Stop the VM of every project whose VM is running.
        #[arg(long)]
        all: bool,
    },
    /// Restart the project's VM, so it mounts the current worktrees.
    Restart {
        #[arg(conflicts_with = "all")]
        project: Option<String>,
        /// Restart the VM of every project whose VM is running.
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        boot: BootArgs,
    },
    /// Create another agent worktree for the project; prints its path.
    #[command(
        after_help = "With --agent or --prompt, the agent starts in the worktree's herdr workspace when the VM next opens it: on `--restart`, `agent project restart` or `agent project start`. The prompt is sent once. Other worktrees keep their own agent, or get $CLI_TOOLBOX_AGENT."
    )]
    Add {
        project: Option<String>,
        /// Restart a running VM right away to mount it.
        #[arg(long)]
        restart: bool,
        #[command(flatten)]
        launch: LaunchArgs,
        /// Do not add the VM to herdr on --restart.
        #[arg(long)]
        no_herdr: bool,
    },
    /// Publish agent work as host review branches, leaving the agent in control.
    #[command(
        after_help = "Publishes committed agent work as ordinary host branches under agent-review/<worktree>/*. Changed guest tips get new snapshots; unchanged tips reuse existing ones. Output shows branch names and commands to view changes or open a separate review worktree. Existing branches, indexes and files are never changed. Normal host branches (excluding agent-review/*) are sent to host-review/<source>/* in the VM for the agent to integrate explicitly. Divergence is a review status, not a sync error. No VM rebuild or flake update is required."
    )]
    Sync {
        #[arg(conflicts_with = "all")]
        project: Option<String>,
        /// Only this worktree.
        #[arg(long, conflicts_with = "all")]
        worktree: Option<String>,
        /// Sync every project whose VM is running.
        #[arg(long)]
        all: bool,
    },
    /// Open the agent's committed changes in lazygit, in a separate host checkout.
    #[command(
        after_help = "Fetches the selected agent's latest snapshot and opens lazygit in its commits view in a separate host review worktree. The agent's checkout is never changed. With no running VM/worktree, offers saved snapshots instead. Repeated reviews reuse matching copies without resetting edits. Requires lazygit on the host and an interactive terminal; no Git commands or branch checkout are needed."
    )]
    Review {
        project: Option<String>,
        /// Only this agent worktree; otherwise select one interactively.
        #[arg(long)]
        worktree: Option<String>,
    },
    /// Save agent work as review snapshots, then remove its worktree.
    #[command(
        after_help = "Preserves committed guest history as host review snapshot branches before removing the worktree. Checks the guest's own index for uncommitted changes; its stale host counterpart does not need updating or merging. Refuses with dirty/detached guest work, failed snapshots, or an unavailable persistent VM. --force removes anyway without these safeguards. Snapshots remain for review after removal. The VM stops when its last worktree is done."
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
pub(super) struct LaunchArgs {
    /// Agent kind to start in the new worktree's herdr workspace, e.g. claude or codex.
    #[arg(long, value_name = "KIND")]
    agent: Option<String>,
    /// First prompt for that agent; `-` reads it from stdin. Without --agent, $CLI_TOOLBOX_AGENT runs it.
    #[arg(long, value_name = "TEXT")]
    prompt: Option<String>,
}

#[derive(Debug, Args)]
struct BootArgs {
    /// Agent kind herdr starts in each guest workspace without an agent of its own, e.g. claude or codex (default: $CLI_TOOLBOX_AGENT).
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
        after_help = "Creates a new worktree and branch and prints the path.\n\nThe worktree is then initialized: a `worktree-init` recipe in the project's justfile runs if there is one (with CLI_TOOLBOX_WORKTREE_SOURCE set to the source checkout), otherwise the install of the package manager whose lockfile is present (npm ci, pnpm install, yarn install, bun install). If herdr is installed, the worktree opens as a focused Herdr workspace. With --agent or --prompt, that agent starts there and gets the prompt.\n\nShell usage:\n\n  cd \"$(cli-toolbox agent worktree create)\"\n  cli-toolbox agent worktree create --agent codex --prompt \"Fix the flaky login test\""
    )]
    Create {
        /// Skip the worktree-init recipe or package install.
        #[arg(long)]
        no_init: bool,
        /// Do not open the worktree in Herdr.
        #[arg(long, conflicts_with_all = ["agent", "prompt"])]
        no_herdr: bool,
        #[command(flatten)]
        launch: LaunchArgs,
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
            launch,
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
                launch.resolve()?.as_ref(),
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
        ProjectCommands::Stop { all: true, .. } => {
            finish(&vm::each_running(&root, vm::stop)?, stderr)
        }
        ProjectCommands::Stop { project, .. } => {
            finish(&vm::stop(&resolve(&root, project)?)?, stderr)
        }
        ProjectCommands::Restart {
            all: true, boot, ..
        } => {
            let options = boot_options(boot);
            let steps = vm::each_running(&root, |project| vm::restart(project, &options, stderr))?;
            finish(&steps, stderr)
        }
        ProjectCommands::Restart { project, boot, .. } => {
            let steps = vm::restart(&resolve(&root, project)?, &boot_options(boot), stderr)?;
            finish(&steps, stderr)
        }
        ProjectCommands::Add {
            project,
            restart,
            launch,
            no_herdr,
        } => {
            let boot = BootArgs {
                agent: None,
                no_herdr,
            };
            let created = vm::add_worktree(
                &resolve(&root, project)?,
                &git::worktrees_root()?,
                restart,
                launch.resolve()?.as_ref(),
                &boot_options(boot),
                stderr,
            )?;
            writeln!(stdout, "{}", created.worktree.path.display())?;
            finish(&created.steps, stderr)
        }
        ProjectCommands::Sync { all: true, .. } => finish(&vm::sync_all(&root, stdout)?, stderr),
        ProjectCommands::Sync {
            project, worktree, ..
        } => {
            let project = resolve(&root, project)?;
            let worktree = worktree
                .map(|name| vm::select_worktree(&project, &name))
                .transpose()?;
            let synced = vm::sync(&project, worktree.as_ref())?;
            vm::write_synced(&synced, stdout)
        }
        ProjectCommands::Review { project, worktree } => {
            let project = resolve(&root, project)?;
            if let Some(target) = agent::review::open(&project, worktree.as_deref(), stderr)? {
                writeln!(stdout, "Review files: {}", target.path.display())?;
            }
            Ok(())
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
        WorktreeCommands::Create {
            no_init,
            no_herdr,
            launch,
        } => {
            let options = agent::Options {
                init: !no_init,
                herdr: !no_herdr,
            };
            agent::create(
                &working_dir()?,
                &git::worktrees_root()?,
                options,
                launch.resolve()?.as_ref(),
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

impl LaunchArgs {
    /// The agent to start in the new worktree, if any.
    fn resolve(self) -> Result<Option<Launch>, Error> {
        launch(
            self.agent,
            self.prompt,
            default_agent(),
            &mut std::io::stdin(),
        )
    }
}

/// `agent` with `prompt`; with only a prompt, the `default` agent runs it.
/// A `-` prompt is read from `stdin`.
fn launch(
    agent: Option<String>,
    prompt: Option<String>,
    default: Option<String>,
    stdin: &mut dyn Read,
) -> Result<Option<Launch>, Error> {
    let prompt = match prompt.as_deref() {
        Some("-") => {
            let mut text = String::new();
            stdin
                .read_to_string(&mut text)
                .map_err(|err| Error::msg(format!("read the prompt from stdin: {err}")))?;
            Some(text.trim().to_string())
        }
        _ => prompt,
    };
    let kind = match (agent, &prompt) {
        (Some(kind), _) => kind,
        (None, Some(_)) => default.ok_or_else(|| {
            Error::msg(format!(
                "--prompt needs an agent: pass --agent KIND or set {AGENT_ENV}"
            ))
        })?,
        (None, None) => return Ok(None),
    };
    Ok(Some(Launch::new(kind, prompt)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_alone_runs_the_default_agent() {
        let mut stdin = "".as_bytes();
        let resolve = |agent: Option<&str>, prompt: Option<&str>, default: Option<&str>| {
            launch(
                agent.map(String::from),
                prompt.map(String::from),
                default.map(String::from),
                &mut "".as_bytes(),
            )
        };
        assert_eq!(resolve(None, None, Some("claude")).unwrap(), None);
        assert_eq!(
            resolve(Some("codex"), None, Some("claude")).unwrap(),
            Some(Launch::new("codex", None))
        );
        assert_eq!(
            resolve(None, Some("Fix it"), Some("claude")).unwrap(),
            Some(Launch::new("claude", Some("Fix it".into())))
        );
        let err = resolve(None, Some("Fix it"), None).unwrap_err();
        assert!(err.to_string().contains("--agent"), "{err}");

        let mut stdin_prompt = "Fix\nthe tests\n".as_bytes();
        assert_eq!(
            launch(Some("pi".into()), Some("-".into()), None, &mut stdin_prompt).unwrap(),
            Some(Launch::new("pi", Some("Fix\nthe tests".into())))
        );
        assert_eq!(
            launch(Some("pi".into()), None, None, &mut stdin).unwrap(),
            Some(Launch::new("pi", None))
        );
    }
}
