mod aws;
mod cli;
mod error;
mod exec;
mod git;
mod kitty;
mod select;
mod ssh;

use std::io::Write;

pub use error::Error;

pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
    cli::run(args, stdout, stderr)
}
