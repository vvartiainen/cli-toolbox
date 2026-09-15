use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::{Error, aws};

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct AwsCmd {
    #[command(subcommand)]
    command: AwsCommands,
}

#[derive(Debug, Subcommand)]
enum AwsCommands {
    /// Select an AWS profile and print an export command.
    #[command(after_help = "Shell usage:\n\n  eval \"$(cli-toolbox aws profile)\"")]
    Profile,
}

impl AwsCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self.command {
            AwsCommands::Profile => {
                let home = home_dir()?;
                aws::select_and_export_profile(&home, stdout, stderr)
            }
        }
    }
}

fn home_dir() -> Result<PathBuf, Error> {
    dirs::home_dir().ok_or_else(|| Error::msg("resolve home directory: home directory not found"))
}
