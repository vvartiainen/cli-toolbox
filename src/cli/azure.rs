use std::io::Write;

use clap::{Args, Subcommand};

use crate::Error;
use crate::azure::{self, AzCli};

#[derive(Debug, Args)]
#[command(subcommand_required = true, subcommand_value_name = "command")]
pub struct AzureCmd {
    #[command(subcommand)]
    command: AzureCommands,
}

#[derive(Debug, Subcommand)]
enum AzureCommands {
    /// Log in to the Azure CLI when there is no valid session.
    #[command(
        after_help = "The tenant is --tenant, else AZURE_TENANT_ID, else your home tenant.\n\nUse --force after activating a PIM role so the new role is used."
    )]
    Login {
        /// Tenant ID or domain (default: $AZURE_TENANT_ID).
        #[arg(long)]
        tenant: Option<String>,
        /// Log in even when the current session is valid.
        #[arg(short, long)]
        force: bool,
    },
    /// Log out, clear cached accounts and print an unset command.
    #[command(after_help = "Shell usage:\n\n  eval \"$(cli-toolbox azure logout)\"")]
    Logout,
    /// Select a subscription, log in if needed, and print export commands.
    #[command(
        after_help = "Also runs `az account set`. Exports AZURE_SUBSCRIPTION_ID, ARM_SUBSCRIPTION_ID and ARM_TENANT_ID.\n\nShell usage:\n\n  eval \"$(cli-toolbox azure subscription)\""
    )]
    Subscription {
        /// Tenant for the first login (default: $AZURE_TENANT_ID).
        #[arg(long)]
        tenant: Option<String>,
    },
    /// Show the logged-in user, subscription and token expiry.
    Status,
}

impl AzureCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        match self.command {
            AzureCommands::Login { tenant, force } => {
                let tenant = azure::tenant_from_env(tenant);
                let status = azure::login(&AzCli::new()?, tenant.as_deref(), force, stderr)?;
                writeln!(stdout, "{status}")?;
            }
            AzureCommands::Logout => {
                let unset = azure::logout(&AzCli::new()?, stderr)?;
                writeln!(stdout, "{unset}")?;
            }
            AzureCommands::Subscription { tenant } => {
                let tenant = azure::tenant_from_env(tenant);
                if let Some(export) =
                    azure::select_subscription(&AzCli::new()?, tenant.as_deref(), stderr)?
                {
                    writeln!(stdout, "{export}")?;
                }
            }
            AzureCommands::Status => {
                let shell = std::env::var("AZURE_SUBSCRIPTION_ID").ok();
                writeln!(
                    stdout,
                    "{}",
                    azure::status(&AzCli::new()?, shell.as_deref())
                )?;
            }
        }
        Ok(())
    }
}
