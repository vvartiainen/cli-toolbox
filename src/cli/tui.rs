use std::io::Write;

use clap::Args;

use crate::{Error, tui};

#[derive(Debug, Args)]
#[command(
    after_help = "Requires an interactive terminal. Use arrows to select, type to search, Enter to open, and Esc to go back. Shell exports and directory changes are shown as instructions to run after exiting."
)]
pub struct TuiCmd;

impl TuiCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        tui::run(stdout, stderr)
    }
}
