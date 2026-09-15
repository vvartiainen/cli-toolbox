use std::io::Write;
use std::path::Path;

use clap::{Args, CommandFactory, ValueEnum};
use clap_complete::{Shell, generate};

use super::Cli;
use crate::Error;

#[derive(Debug, Args)]
pub struct CompletionCmd {
    /// Shell to generate a tab completion script for.
    #[arg(short = 'c', long = "code", value_enum)]
    code: Option<CompletionShell>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

impl CompletionCmd {
    pub fn run(self, stdout: &mut dyn Write, _stderr: &mut dyn Write) -> Result<(), Error> {
        let shell = match self.code {
            Some(shell) => shell,
            None => login_shell()?,
        };

        if self.code.is_none() {
            writeln!(stdout, "{}", source_command(shell))?;
            return Ok(());
        }

        generate_script(shell, stdout);
        Ok(())
    }
}

fn generate_script(shell: CompletionShell, stdout: &mut dyn Write) {
    let mut command = Cli::command();
    match shell {
        CompletionShell::Bash => generate(Shell::Bash, &mut command, "cli-toolbox", stdout),
        CompletionShell::Zsh => generate(Shell::Zsh, &mut command, "cli-toolbox", stdout),
        CompletionShell::Fish => generate(Shell::Fish, &mut command, "cli-toolbox", stdout),
    }
}

fn source_command(shell: CompletionShell) -> String {
    match shell {
        CompletionShell::Bash | CompletionShell::Zsh => {
            format!("source <(cli-toolbox completion -c {})", shell_name(shell))
        }
        CompletionShell::Fish => "cli-toolbox completion -c fish | source".into(),
    }
}

fn login_shell() -> Result<CompletionShell, Error> {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let name = Path::new(&shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    match name {
        "bash" => Ok(CompletionShell::Bash),
        "zsh" => Ok(CompletionShell::Zsh),
        "fish" => Ok(CompletionShell::Fish),
        _ => Err(Error::msg(format!(
            "unsupported login shell {name:?}; pass --code bash|zsh|fish"
        ))),
    }
}

fn shell_name(shell: CompletionShell) -> &'static str {
    match shell {
        CompletionShell::Bash => "bash",
        CompletionShell::Zsh => "zsh",
        CompletionShell::Fish => "fish",
    }
}

#[cfg(test)]
pub(super) fn generate_to_string(shell: &str) -> Result<String, Error> {
    let shell = match shell {
        "bash" => CompletionShell::Bash,
        "zsh" => CompletionShell::Zsh,
        "fish" => CompletionShell::Fish,
        other => return Err(Error::msg(format!("unsupported shell {other}"))),
    };
    let mut buf = Vec::new();
    generate_script(shell, &mut buf);
    Ok(String::from_utf8(buf).unwrap_or_default())
}
