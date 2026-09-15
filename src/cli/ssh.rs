use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::{Error, ssh};

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct SshCmd {
    #[command(subcommand)]
    command: SshCommands,
}

#[derive(Debug, Subcommand)]
enum SshCommands {
    /// Select an SSH host from config and connect with kitten ssh.
    Connect,
}

impl SshCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self.command {
            SshCommands::Connect => {
                let home = home_dir()?;
                ssh::select_and_connect_host(&home, stdout, stderr)
            }
        }
    }
}

fn home_dir() -> Result<PathBuf, Error> {
    dirs::home_dir().ok_or_else(|| Error::msg("resolve home directory: home directory not found"))
}
