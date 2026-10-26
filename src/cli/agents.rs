use crate::{
    Error,
    agents::{self, Agent, Environment},
};
use clap::{Args, Subcommand, ValueEnum};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Args)]
pub struct AgentsCmd {
    #[command(subcommand)]
    command: Option<AgentsCommand>,
}
#[derive(Debug, Subcommand)]
enum AgentsCommand {
    /// Show which catalog MCP servers and skills this repository uses.
    Status,
    /// Turn one MCP server or skill on or off for this repository.
    Set {
        #[arg(value_enum)]
        kind: Kind,
        name: String,
        #[arg(value_enum)]
        choice: Toggle,
    },
    /// Start a coding agent with this repository's MCP servers and skills.
    Launch {
        /// claude, codex, opencode, copilot or pi.
        agent: String,
        /// The agent executable; its command name on PATH by default.
        #[arg(long, value_name = "PATH")]
        bin: Option<PathBuf>,
        /// Arguments for the agent.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Install locked skill sources or explicitly update their revisions.
    Library {
        #[command(subcommand)]
        command: LibraryCommand,
    },
}
#[derive(Debug, Subcommand)]
enum LibraryCommand {
    Install,
    Update,
}
#[derive(Clone, Debug, ValueEnum)]
enum Kind {
    Mcp,
    Skills,
}
#[derive(Clone, Debug, ValueEnum)]
enum Toggle {
    Enable,
    Disable,
}
impl AgentsCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        if let Some(AgentsCommand::Launch { agent, bin, args }) = self.command {
            return launch(&agent, bin, args, stderr);
        }
        let env = Environment::discover()?;
        match self.command {
            Some(AgentsCommand::Library { command }) => {
                env.install(matches!(command, LibraryCommand::Update))?;
                writeln!(
                    stdout,
                    "Library installed. Skill revisions: {}",
                    env.library.join("rulesync.lock").display()
                )?;
                Ok(())
            }
            command => env.with_project(|project| match command {
                Some(AgentsCommand::Status) => project.status(stdout),
                Some(AgentsCommand::Set { kind, name, choice }) => project.set_choice(
                    kind.to_possible_value().unwrap().get_name(),
                    &name,
                    matches!(choice, Toggle::Enable),
                ),
                None => project.interactive(stdout),
                Some(AgentsCommand::Library { .. } | AgentsCommand::Launch { .. }) => {
                    unreachable!()
                }
            }),
        }
    }
}
/// Replace this process with the agent, given the chosen tools. Problems
/// finding them are reported, and the agent starts without them.
fn launch(
    name: &str,
    bin: Option<PathBuf>,
    args: Vec<String>,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let agent = Agent::parse(name).ok_or_else(|| {
        Error::msg(format!(
            "unknown agent: {name}; use claude, codex, opencode, copilot or pi"
        ))
    })?;
    let mut command = Command::new(bin.unwrap_or_else(|| agent.name().into()));
    if agent.starts_session(&args) {
        match agents::launch_tools(agent) {
            Ok(Some(tools)) => {
                for note in &tools.notes {
                    writeln!(stderr, "ct agents: {note}")?;
                }
                command.args(tools.args);
            }
            Ok(None) => {}
            Err(error) => writeln!(
                stderr,
                "ct agents: {error}; starting {name} without chosen tools"
            )?,
        }
    }
    stderr.flush()?;
    let error = command.args(args).exec();
    Err(Error::msg(format!(
        "start {}: {error}",
        command.get_program().display()
    )))
}
#[cfg(test)]
mod tests {
    use crate::cli::{Cli, run};
    use clap::Parser;
    #[test]
    fn commands_appear_in_help_and_parse() {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run(&["agents".into(), "--help".into()], &mut out, &mut err).unwrap();
        let help = String::from_utf8(out).unwrap();
        for name in ["status", "set", "launch", "library"] {
            assert!(help.contains(name), "{help}");
        }
        assert!(err.is_empty());
        for args in [
            vec!["agents"],
            vec!["agents", "status"],
            vec!["agents", "set", "mcp", "nixos", "enable"],
            vec!["agents", "set", "skills", "review", "disable"],
            vec!["agents", "launch", "claude"],
            // Agent options pass through, including ones that look like ours.
            vec![
                "agents", "launch", "codex", "--bin", "/x/codex", "-c", "a=1", "--help",
            ],
            vec!["agents", "library", "install"],
            vec!["agents", "library", "update"],
        ] {
            Cli::try_parse_from(std::iter::once("ct").chain(args)).unwrap();
        }
        assert!(Cli::try_parse_from(["ct", "agents", "set", "mcp", "nixos", "bad"]).is_err());
        assert!(Cli::try_parse_from(["ct", "agents", "set", "targets", "pi", "enable"]).is_err());
    }
}
