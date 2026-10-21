use crate::{Error, agents::Environment};
use clap::{Args, Subcommand, ValueEnum};
use std::io::Write;

#[derive(Debug, Args)]
pub struct AgentsCmd {
    /// Write portable project choices instead of personal choices.
    #[arg(long, global = true)]
    shared: bool,
    #[command(subcommand)]
    command: Option<AgentsCommand>,
}
#[derive(Debug, Subcommand)]
enum AgentsCommand {
    /// Show effective choices and file drift without running Rulesync.
    Status,
    /// Preview or apply selected tools. Never downloads skills.
    Sync {
        #[arg(long)]
        dry_run: bool,
        /// Adopt an existing MCP section, preserving unrelated servers.
        #[arg(long, value_name = "TARGET")]
        adopt_mcp: Vec<String>,
    },
    /// Change one selection without synchronizing.
    Set {
        #[arg(value_enum)]
        kind: Kind,
        name: String,
        #[arg(value_enum)]
        choice: Choice,
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
    Targets,
    Mcp,
    Skills,
}
#[derive(Clone, Debug, ValueEnum)]
enum Choice {
    Inherit,
    Enable,
    Disable,
}
impl AgentsCmd {
    pub fn run(self, stdout: &mut dyn Write, _stderr: &mut dyn Write) -> Result<(), Error> {
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
                Some(AgentsCommand::Sync { dry_run, adopt_mcp }) => {
                    project.sync(dry_run, &adopt_mcp, stdout).map(|_| ())
                }
                Some(AgentsCommand::Set { kind, name, choice }) => project.set_choice(
                    kind.to_possible_value().unwrap().get_name(),
                    &name,
                    choice.to_possible_value().unwrap().get_name(),
                    self.shared,
                ),
                None => project.interactive(self.shared, stdout),
                Some(AgentsCommand::Library { .. }) => unreachable!(),
            }),
        }
    }
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
        for name in ["status", "sync", "set", "library", "--shared"] {
            assert!(help.contains(name), "{help}");
        }
        assert!(err.is_empty());
        for args in [
            vec!["agents"],
            vec!["agents", "status"],
            vec!["agents", "set", "mcp", "nixos", "enable", "--shared"],
            vec![
                "agents",
                "sync",
                "--dry-run",
                "--adopt-mcp",
                "codexcli",
                "--adopt-mcp",
                "pi",
            ],
            vec!["agents", "library", "install"],
            vec!["agents", "library", "update"],
        ] {
            Cli::try_parse_from(std::iter::once("ct").chain(args)).unwrap();
        }
        assert!(Cli::try_parse_from(["ct", "agents", "set", "mcp", "nixos", "bad"]).is_err());
    }
}
