use std::io::Write;

use clap::Args;

use crate::{Error, tui};

#[derive(Debug, Args)]
#[command(
    after_help = "Requires an interactive terminal. Press a/i/t (or [ and ]) to switch between the AWS, Agents and Terminal tabs, a number to run an action, / to search all actions, ? or F1 for all keys, and Esc to go back.\n\nShell exports and directory changes are shown as instructions to run after exiting. To apply them directly, capture --print-shell output with eval:\n\n  ct() { local cmd; cmd=\"$(cli-toolbox tui --print-shell)\" || return; [ -n \"$cmd\" ] && eval \"$cmd\"; }"
)]
pub struct TuiCmd {
    /// After an AWS profile or worktree action succeeds, exit and print the shell command to stdout for eval.
    #[arg(long)]
    print_shell: bool,
}

impl TuiCmd {
    pub fn run(self, stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
        tui::run(self.print_shell, stdout, stderr)
    }
}
