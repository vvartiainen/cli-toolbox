use crate::{Error, agents::Environment};
use clap::{Args, Subcommand, ValueEnum};
use std::io::Write;

#[derive(Debug, Args)]
pub struct AgentsCmd {
    #[command(subcommand)]
    command: Option<AgentsCommand>,
}
#[derive(Debug, Subcommand)]
enum AgentsCommand {
    /// Show the repository's choices and whether its files match them.
    Status,
    /// Turn one target, MCP server or skill on or off, without syncing.
    Set {
        #[arg(value_enum)]
        kind: Kind,
        name: String,
        #[arg(value_enum)]
        choice: Toggle,
    },
    /// Write the chosen MCP servers and skills into the targets' project files.
    Sync {
        /// List the changes without making them.
        #[arg(long)]
        dry_run: bool,
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
enum Toggle {
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
                Some(AgentsCommand::Set { kind, name, choice }) => project.set_choice(
                    kind.to_possible_value().unwrap().get_name(),
                    &name,
                    matches!(choice, Toggle::Enable),
                ),
                Some(AgentsCommand::Sync { dry_run }) => project.sync(dry_run, stdout),
                None => project.interactive(stdout),
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
        for name in ["status", "set", "sync", "library"] {
            assert!(help.contains(name), "{help}");
        }
        assert!(err.is_empty());
        for args in [
            vec!["agents"],
            vec!["agents", "status"],
            vec!["agents", "set", "targets", "claude", "enable"],
            vec!["agents", "set", "mcp", "nixos", "enable"],
            vec!["agents", "set", "skills", "review", "disable"],
            vec!["agents", "sync"],
            vec!["agents", "sync", "--dry-run"],
            vec!["agents", "library", "install"],
            vec!["agents", "library", "update"],
        ] {
            Cli::try_parse_from(std::iter::once("ct").chain(args)).unwrap();
        }
        assert!(Cli::try_parse_from(["ct", "agents", "set", "mcp", "nixos", "bad"]).is_err());
        assert!(Cli::try_parse_from(["ct", "agents", "launch", "claude"]).is_err());
    }
}
