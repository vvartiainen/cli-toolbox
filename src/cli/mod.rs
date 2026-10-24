mod agent;
mod agents;
mod aws;
mod azure;
mod complete;
mod git;
mod kitty;
mod ssh;
mod tui;

use std::io::Write;

use clap::{Parser, Subcommand};

use self::agent::AgentCmd;
use self::agents::AgentsCmd;
use self::aws::AwsCmd;
use self::azure::AzureCmd;
use self::complete::CompletionCmd;
use self::git::GitCmd;
use self::kitty::KittyCmd;
use self::ssh::SshCmd;
use self::tui::TuiCmd;
use crate::Error;

#[derive(Debug, Parser)]
#[command(
    name = "cli-toolbox",
    about = "cli-toolbox helps with small workflow tasks.",
    subcommand_value_name = "command",
    subcommand_required = true
)]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Open the interactive tool launcher.
    Tui(TuiCmd),
    /// AWS helpers.
    Aws(AwsCmd),
    /// Azure CLI helpers.
    #[command(alias = "az")]
    Azure(AzureCmd),
    /// AI agent helpers.
    Agent(AgentCmd),
    /// Project MCP and skill selections through Rulesync.
    Agents(AgentsCmd),
    /// Git helpers (hidden alias for `agent worktree`).
    #[command(hide = true)]
    Git(GitCmd),
    /// Kitty helpers.
    Kitty(KittyCmd),
    /// SSH helpers.
    #[command(name = "ssh")]
    Ssh(SshCmd),
    /// Generate shell completion scripts.
    Completion(CompletionCmd),
}

pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
    let argv = if args.is_empty() {
        vec!["cli-toolbox".into(), "--help".into()]
    } else {
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push("cli-toolbox".into());
        argv.extend(args.iter().cloned());
        argv
    };

    match Cli::try_parse_from(argv) {
        Ok(cli) => cli.command.run(stdout, stderr),
        Err(err) => {
            let rendered = err.render().to_string();
            if err.use_stderr() {
                write!(stderr, "{rendered}")?;
                Err(Error::displayed(err.to_string()))
            } else {
                write!(stdout, "{rendered}")?;
                Ok(())
            }
        }
    }
}

impl Commands {
    fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self {
            Self::Tui(cmd) => cmd.run(stdout, stderr),
            Self::Aws(cmd) => cmd.run(stdout, stderr),
            Self::Azure(cmd) => cmd.run(stdout, stderr),
            Self::Agent(cmd) => cmd.run(stdout, stderr),
            Self::Agents(cmd) => cmd.run(stdout, stderr),
            Self::Git(cmd) => cmd.run(stdout, stderr),
            Self::Kitty(cmd) => cmd.run(stdout, stderr),
            Self::Ssh(cmd) => cmd.run(stdout, stderr),
            Self::Completion(cmd) => cmd.run(stdout, stderr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, run};
    use crate::cli::complete::generate_to_string;
    use clap::CommandFactory;

    fn run_args(args: &[&str]) -> (Result<(), crate::Error>, String, String) {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = run(&args, &mut stdout, &mut stderr);
        (
            result,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn prints_tui_help_without_opening_terminal() {
        let (result, stdout, stderr) = run_args(&["tui", "--help"]);
        result.unwrap();
        assert!(stdout.contains("Usage: cli-toolbox tui"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn completion_includes_tui() {
        let script = generate_to_string("bash").unwrap();
        assert!(script.contains("tui"));
    }

    #[test]
    fn prints_root_usage_without_args() {
        let (result, stdout, stderr) = run_args(&[]);
        result.unwrap();
        assert!(stdout.contains("Usage: cli-toolbox <command>"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_kitty_help() {
        let (result, stdout, stderr) = run_args(&["kitty", "--help"]);
        result.unwrap();
        assert!(stdout.contains("Usage: cli-toolbox kitty"), "{stdout}");
        assert!(stdout.contains("select-session"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn rejects_kitty_without_subcommand() {
        let (result, stdout, stderr) = run_args(&["kitty"]);
        let err = result.unwrap_err();
        assert!(err.to_string().contains("subcommand"), "{}", err);
        assert!(
            stderr.contains("Usage: cli-toolbox kitty <command>"),
            "{stderr}"
        );
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn prints_kitty_select_session_help() {
        let (result, stdout, stderr) = run_args(&["kitty", "select-session", "--help"]);
        result.unwrap();
        assert!(stdout.contains("Usage: cli-toolbox kitty"), "{stdout}");
        assert!(stdout.contains("select-session"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_ssh_help() {
        let (result, stdout, stderr) = run_args(&["ssh", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox ssh <command>"),
            "{stdout}"
        );
        assert!(stdout.contains("connect"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_ssh_connect_help() {
        let (result, stdout, stderr) = run_args(&["ssh", "connect", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox ssh connect"),
            "{stdout}"
        );
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_agent_help() {
        let (result, stdout, stderr) = run_args(&["agent", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox agent <command>"),
            "{stdout}"
        );
        assert!(stdout.contains("worktree"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_sandbox_project_help() {
        for command in [
            "list", "create", "setup", "start", "stop", "restart", "add", "sync", "done",
        ] {
            let (result, stdout, stderr) = run_args(&["agent", "project", command, "--help"]);
            result.unwrap();
            let usage = format!("Usage: cli-toolbox agent project {command}");
            assert!(stdout.contains(&usage), "{stdout}");
            assert!(stderr.is_empty(), "{stderr}");
        }
        let (_, stdout, _) = run_args(&["agent", "project", "--help"]);
        for text in [
            "~/.agent-sandbox/projects/",
            "worktrees.nix",
            "project done",
        ] {
            assert!(stdout.contains(text), "missing {text}: {stdout}");
        }
        let (_, stdout, _) = run_args(&["agent", "project", "start", "--help"]);
        for text in ["--agent", "--no-herdr"] {
            assert!(stdout.contains(text), "missing {text}: {stdout}");
        }
    }

    #[test]
    fn sandbox_project_commands_take_all_running_vms() {
        for command in ["stop", "restart", "sync"] {
            let (_, stdout, _) = run_args(&["agent", "project", command, "--help"]);
            assert!(stdout.contains("--all"), "{command}: {stdout}");
            let (result, _, _) = run_args(&["agent", "project", command, "demo", "--all"]);
            let err = result.unwrap_err();
            assert!(err.to_string().contains("cannot be used"), "{err}");
        }
        let (result, _, _) = run_args(&["agent", "project", "sync", "--all", "--worktree", "w"]);
        assert!(result.is_err());
    }

    #[test]
    fn project_sync_help_explains_bidirectional_review_without_branch_updates() {
        let (result, stdout, stderr) = run_args(&["agent", "project", "sync", "--help"]);
        result.unwrap();
        for text in [
            "sandbox-review/<source>",
            "host-review/<source>",
            "Divergence is a review status",
        ] {
            assert!(stdout.contains(text), "missing {text}: {stdout}");
        }
        assert!(!stdout.contains("fast-forwards"));
        assert!(stderr.is_empty());
    }

    #[test]
    fn prints_agent_worktree_help() {
        let (result, stdout, stderr) = run_args(&["agent", "worktree", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox agent worktree"),
            "{stdout}"
        );
        for subcommand in ["create", "list", "cleanup"] {
            assert!(stdout.contains(subcommand), "{stdout}");
        }
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn rejects_agent_worktree_without_subcommand() {
        let (result, stdout, stderr) = run_args(&["agent", "worktree"]);
        let err = result.unwrap_err();
        assert!(err.to_string().contains("subcommand"), "{err}");
        assert!(
            stderr.contains("Usage: cli-toolbox agent worktree <command>"),
            "{stderr}"
        );
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn prints_agent_worktree_create_help() {
        let (result, stdout, stderr) = run_args(&["agent", "worktree", "create", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox agent worktree create"),
            "{stdout}"
        );
        assert!(
            stdout.contains(r#"cd "$(cli-toolbox agent worktree create)""#),
            "{stdout}"
        );
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_agent_worktree_list_help() {
        let (result, stdout, stderr) = run_args(&["agent", "worktree", "list", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox agent worktree list"),
            "{stdout}"
        );
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_agent_worktree_cleanup_help() {
        let (result, stdout, stderr) = run_args(&["agent", "worktree", "cleanup", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox agent worktree cleanup"),
            "{stdout}"
        );
        assert!(stdout.contains("--all"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn rejects_unknown_agent_subcommand() {
        let (result, stdout, stderr) = run_args(&["agent", "wat"]);
        let err = result.unwrap_err();
        assert!(err.to_string().contains("subcommand"), "{err}");
        assert!(
            stderr.contains("Usage: cli-toolbox agent <command>"),
            "{stderr}"
        );
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn git_agent_worktree_alias_still_parses_and_is_hidden() {
        let (result, stdout, stderr) = run_args(&["git", "agent-worktree", "cleanup", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox git agent-worktree cleanup"),
            "{stdout}"
        );
        assert!(stderr.is_empty(), "{stderr}");

        let (result, root_help, _) = run_args(&["--help"]);
        result.unwrap();
        assert!(root_help.contains("agent"), "{root_help}");
        assert!(!root_help.contains("Git helpers"), "{root_help}");
    }

    #[test]
    fn prints_aws_profile_help() {
        let (result, stdout, stderr) = run_args(&["aws", "profile", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox aws profile"),
            "{stdout}"
        );
        assert!(
            stdout.contains(r#"eval "$(cli-toolbox aws profile)""#),
            "{stdout}"
        );
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn rejects_kitty_select_session_positional_args() {
        let (result, stdout, stderr) = run_args(&["kitty", "select-session", "extra"]);
        let err = result.unwrap_err();
        assert!(err.to_string().contains("unexpected argument"), "{err}");
        assert!(stderr.contains("Usage: cli-toolbox kitty"), "{stderr}");
        assert!(stderr.contains("select-session"), "{stderr}");
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn rejects_unknown_kitty_select_session_flag() {
        let (result, stdout, stderr) = run_args(&["kitty", "select-session", "--wat"]);
        let err = result.unwrap_err();
        assert!(err.to_string().contains("unexpected argument"), "{err}");
        assert!(stderr.contains("Usage: cli-toolbox kitty"), "{stderr}");
        assert!(stderr.contains("select-session"), "{stderr}");
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn rejects_unknown_aws_subcommand() {
        let (result, stdout, stderr) = run_args(&["aws", "wat"]);
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("unrecognized subcommand")
                || err.to_string().contains("subcommand"),
            "{err}"
        );
        assert!(
            stderr.contains("Usage: cli-toolbox aws <command>"),
            "{stderr}"
        );
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn rejects_unknown_ssh_subcommand() {
        let (result, stdout, stderr) = run_args(&["ssh", "wat"]);
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("unrecognized subcommand")
                || err.to_string().contains("subcommand"),
            "{err}"
        );
        assert!(
            stderr.contains("Usage: cli-toolbox ssh <command>"),
            "{stderr}"
        );
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn prints_completion_help() {
        let (result, stdout, stderr) = run_args(&["completion", "--help"]);
        result.unwrap();
        assert!(stdout.contains("Usage: cli-toolbox completion"), "{stdout}");
        assert!(stdout.contains("--code"), "{stdout}");
        assert!(stdout.contains("tab completion"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_zsh_completion_script() {
        let (result, stdout, stderr) = run_args(&["completion", "-c", "zsh"]);
        result.unwrap();
        assert!(stdout.contains("#compdef cli-toolbox"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn generated_completion_includes_root_commands() {
        let stdout = generate_to_string("bash").unwrap();
        for command in ["agent", "aws", "azure", "completion", "kitty", "ssh", "tui"] {
            assert!(stdout.contains(command), "missing {command}: {stdout}");
        }
    }

    #[test]
    fn generated_completion_includes_completion_shells() {
        let stdout = generate_to_string("bash").unwrap();
        for shell in ["bash", "zsh", "fish"] {
            assert!(stdout.contains(shell), "missing {shell}: {stdout}");
        }
    }

    #[test]
    fn generated_completion_includes_agent_worktree_subcommands() {
        let stdout = generate_to_string("bash").unwrap();
        for command in [
            "worktree", "create", "list", "cleanup", "project", "start", "restart", "add", "sync",
            "done",
        ] {
            assert!(stdout.contains(command), "missing {command}: {stdout}");
        }
    }

    #[test]
    fn generated_completion_includes_cleanup_flags() {
        let stdout = generate_to_string("bash").unwrap();
        assert!(stdout.contains("--all"), "{stdout}");
        assert!(stdout.contains("--help"), "{stdout}");
    }

    #[test]
    fn rejects_unknown_completion_shell() {
        let (result, stdout, _stderr) = run_args(&["completion", "powershell"]);
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("powershell")
                || err.to_string().contains("unexpected argument"),
            "{err}"
        );
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn command_factory_builds() {
        let _ = Cli::command();
    }
}
