use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    match cli_toolbox::run(&args, &mut stdout, &mut stderr) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            if !err.was_displayed() {
                let _ = writeln!(stderr, "error: {err}");
            }
            ExitCode::from(1)
        }
    }
}
