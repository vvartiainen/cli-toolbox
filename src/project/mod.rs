use std::fmt;
use std::path::Path;

use crate::{Error, exec};

/// Justfile recipe a project defines to prepare a fresh worktree.
pub(crate) const INIT_RECIPE: &str = "worktree-init";

/// Environment variable that tells the init recipe which checkout the
/// worktree was created from, for example to copy untracked `.env` files.
pub(crate) const SOURCE_ENV: &str = "CLI_TOOLBOX_WORKTREE_SOURCE";

const JUSTFILES: [&str; 3] = ["justfile", "Justfile", ".justfile"];

/// Package manager installs, checked in order. The first marker file found
/// in the project root wins, so lockfiles take precedence over a bare
/// `package.json`.
const INSTALLS: [(&str, &str, &[&str]); 6] = [
    ("pnpm-lock.yaml", "pnpm", &["install", "--frozen-lockfile"]),
    ("yarn.lock", "yarn", &["install"]),
    ("bun.lock", "bun", &["install"]),
    ("bun.lockb", "bun", &["install"]),
    ("package-lock.json", "npm", &["ci"]),
    ("package.json", "npm", &["install"]),
];

/// The command that initializes a project checkout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InitCommand {
    program: &'static str,
    args: Vec<&'static str>,
}

impl fmt::Display for InitCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.program)?;
        for arg in &self.args {
            write!(f, " {arg}")?;
        }
        Ok(())
    }
}

/// Find how to initialize `dir`: the project's `worktree-init` just recipe,
/// or else an install for the package manager whose lockfile is present.
pub(crate) fn detect_init(dir: &Path) -> Option<InitCommand> {
    detect_init_with(dir, has_just_recipe)
}

fn detect_init_with(dir: &Path, has_recipe: impl Fn(&Path, &str) -> bool) -> Option<InitCommand> {
    let has_justfile = JUSTFILES.iter().any(|name| dir.join(name).is_file());
    if has_justfile && has_recipe(dir, INIT_RECIPE) {
        return Some(InitCommand {
            program: "just",
            args: vec![INIT_RECIPE],
        });
    }
    INSTALLS
        .iter()
        .find(|(marker, _, _)| dir.join(marker).is_file())
        .map(|(_, program, args)| InitCommand {
            program,
            args: args.to_vec(),
        })
}

fn has_just_recipe(dir: &Path, recipe: &str) -> bool {
    if exec::look_path("just").is_none() {
        return false;
    }
    exec::combined_output("just", &["--summary"], Some(dir))
        .is_ok_and(|summary| summary.split_whitespace().any(|name| name == recipe))
}

/// Run `command` in `dir`. Its output streams to stderr so that long installs
/// show progress, and stdout stays free for the worktree path.
pub(crate) fn run_init(command: &InitCommand, dir: &Path, source: &Path) -> Result<(), Error> {
    exec::require(command.program)?;
    exec::run_on_stderr_in(
        command.program,
        &command.args,
        Some(dir),
        &[(SOURCE_ENV, source.as_os_str())],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(files: &[&str], recipe: bool) -> Option<String> {
        let temp = tempfile::tempdir().unwrap();
        for file in files {
            std::fs::write(temp.path().join(file), "").unwrap();
        }
        detect_init_with(temp.path(), |_, name| recipe && name == INIT_RECIPE)
            .map(|command| command.to_string())
    }

    #[test]
    fn justfile_recipe_wins_over_lockfiles() {
        assert_eq!(
            detect(&["justfile", "package-lock.json"], true).as_deref(),
            Some("just worktree-init")
        );
    }

    #[test]
    fn falls_back_to_the_lockfile_package_manager() {
        assert_eq!(
            detect(&["justfile", "package.json", "package-lock.json"], false).as_deref(),
            Some("npm ci")
        );
        assert_eq!(
            detect(&["package.json", "pnpm-lock.yaml"], false).as_deref(),
            Some("pnpm install --frozen-lockfile")
        );
        assert_eq!(
            detect(&["package.json"], false).as_deref(),
            Some("npm install")
        );
    }

    #[test]
    fn recipe_without_a_justfile_and_empty_projects_are_ignored() {
        assert_eq!(detect(&[], true), None);
        assert_eq!(detect(&["Cargo.toml"], false), None);
    }

    #[cfg(unix)]
    #[test]
    fn init_runs_in_the_worktree_with_the_source_path() {
        let worktree = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let command = InitCommand {
            program: "sh",
            args: vec!["-c", "printf %s \"$CLI_TOOLBOX_WORKTREE_SOURCE\" > seen"],
        };
        run_init(&command, worktree.path(), source.path()).unwrap();
        let seen = std::fs::read_to_string(worktree.path().join("seen")).unwrap();
        assert_eq!(Path::new(&seen), source.path());
    }
}
