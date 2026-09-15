use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::{Error, kitty};

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct KittyCmd {
    #[command(subcommand)]
    command: KittyCommands,
}

#[derive(Debug, Subcommand)]
enum KittyCommands {
    /// Select a kitty session file and launch it.
    #[command(name = "select-session")]
    SelectSession,
}

impl KittyCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self.command {
            KittyCommands::SelectSession => {
                let home = home_dir()?;
                kitty::select_and_launch_session(&home, stdout, stderr)
            }
        }
    }
}

fn home_dir() -> Result<PathBuf, Error> {
    dirs::home_dir().ok_or_else(|| Error::msg("resolve home directory: home directory not found"))
}
