mod aws;
mod complete;
mod git;
mod kitty;
mod ssh;

use std::io::Write;

use clap::{Parser, Subcommand};

use self::aws::AwsCmd;
use self::complete::CompletionCmd;
use self::git::GitCmd;
use self::kitty::KittyCmd;
use self::ssh::SshCmd;
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
    /// AWS helpers.
    Aws(AwsCmd),
    /// Git helpers.
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
            Self::Aws(cmd) => cmd.run(stdout, stderr),
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
    fn prints_git_help() {
        let (result, stdout, stderr) = run_args(&["git", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox git <command>"),
            "{stdout}"
        );
        assert!(stdout.contains("agent-worktree"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_git_agent_worktree_help() {
        let (result, stdout, stderr) = run_args(&["git", "agent-worktree", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox git agent-worktree"),
            "{stdout}"
        );
        for subcommand in ["create", "list", "cleanup"] {
            assert!(stdout.contains(subcommand), "{stdout}");
        }
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn rejects_git_agent_worktree_without_subcommand() {
        let (result, stdout, stderr) = run_args(&["git", "agent-worktree"]);
        let err = result.unwrap_err();
        assert!(err.to_string().contains("subcommand"), "{err}");
        assert!(
            stderr.contains("Usage: cli-toolbox git agent-worktree <command>"),
            "{stderr}"
        );
        assert!(stdout.is_empty(), "{stdout}");
    }

    #[test]
    fn prints_git_agent_worktree_create_help() {
        let (result, stdout, stderr) = run_args(&["git", "agent-worktree", "create", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox git agent-worktree create"),
            "{stdout}"
        );
        assert!(
            stdout.contains(r#"cd "$(cli-toolbox git agent-worktree create)""#),
            "{stdout}"
        );
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_git_agent_worktree_list_help() {
        let (result, stdout, stderr) = run_args(&["git", "agent-worktree", "list", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox git agent-worktree list"),
            "{stdout}"
        );
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn prints_git_agent_worktree_cleanup_help() {
        let (result, stdout, stderr) = run_args(&["git", "agent-worktree", "cleanup", "--help"]);
        result.unwrap();
        assert!(
            stdout.contains("Usage: cli-toolbox git agent-worktree cleanup"),
            "{stdout}"
        );
        assert!(stdout.contains("--all"), "{stdout}");
        assert!(stderr.is_empty(), "{stderr}");
    }

    #[test]
    fn rejects_unknown_git_subcommand() {
        let (result, stdout, stderr) = run_args(&["git", "wat"]);
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("unrecognized subcommand")
                || err.to_string().contains("subcommand"),
            "{err}"
        );
        assert!(
            stderr.contains("Usage: cli-toolbox git <command>"),
            "{stderr}"
        );
        assert!(stdout.is_empty(), "{stdout}");
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
        for command in ["aws", "completion", "git", "kitty", "ssh"] {
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
    fn generated_completion_includes_git_agent_worktree_subcommands() {
        let stdout = generate_to_string("bash").unwrap();
        for command in ["create", "list", "cleanup"] {
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
