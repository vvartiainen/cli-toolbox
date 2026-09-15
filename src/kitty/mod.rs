use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Error, exec, select};

const SESSION_GLOB: &str = "*.kitty-session";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub path: PathBuf,
}

pub fn select_and_launch_session(
    home: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let paths = find_sessions(home)?;
    if paths.is_empty() {
        return Err(Error::msg(format!(
            "no kitty session files found matching \"{}\"",
            home.join(SESSION_GLOB).display()
        )));
    }

    let Some(selected) = select_session(home, &paths)? else {
        return Ok(());
    };
    launch_session(&selected, stdout, stderr)
}

pub fn find_sessions(home: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut matches = Vec::new();
    let entries = match std::fs::read_dir(home) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(Error::msg(format!("glob kitty session files: {err}")));
        }
    };

    for entry in entries {
        let entry = entry.map_err(|err| Error::msg(format!("glob kitty session files: {err}")))?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if glob_match(SESSION_GLOB, name) && path.is_file() {
            matches.push(path);
        }
    }
    matches.sort();
    Ok(matches)
}

pub fn build_choices(home: &Path, paths: &[PathBuf]) -> Vec<Choice> {
    paths
        .iter()
        .map(|path| {
            let label = path
                .strip_prefix(home)
                .ok()
                .filter(|rel| *rel != Path::new(".") && !rel.starts_with(".."))
                .map(|rel| Path::new("~").join(rel).display().to_string())
                .unwrap_or_else(|| path.display().to_string());
            Choice {
                label,
                path: path.clone(),
            }
        })
        .collect()
}

fn select_session(home: &Path, paths: &[PathBuf]) -> Result<Option<PathBuf>, Error> {
    let choices = build_choices(home, paths);
    let lines: Vec<String> = choices.iter().map(|choice| choice.label.clone()).collect();
    let Some(index) = select::pick("kitty session> ", &lines, "40%")? else {
        return Ok(None);
    };
    choices
        .get(index)
        .map(|choice| Some(choice.path.clone()))
        .ok_or_else(|| Error::msg("selected kitty session was not found"))
}

fn launch_session(
    path: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let cmd_name = remote_control_executable()?;
    exec::run_captured(
        cmd_name,
        &["@", "action", "goto_session", &path.to_string_lossy()],
        None,
        stdout,
        stderr,
    )
    .map_err(|err| {
        Error::msg(format!(
            "launch kitty session \"{}\" via remote control: {err}",
            path.display()
        ))
    })
}

fn remote_control_executable() -> Result<&'static str, Error> {
    exec::require_any(&["kitten", "kitty"])
}

fn glob_match(pattern: &str, name: &str) -> bool {
    glob_match_bytes(pattern.as_bytes(), name.as_bytes())
}

fn glob_match_bytes(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.first() {
        None => name.is_empty(),
        Some(&b'*') => {
            glob_match_bytes(&pattern[1..], name)
                || (!name.is_empty() && glob_match_bytes(pattern, &name[1..]))
        }
        Some(&b'?') => !name.is_empty() && glob_match_bytes(&pattern[1..], &name[1..]),
        Some(&ch) => name.first() == Some(&ch) && glob_match_bytes(&pattern[1..], &name[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, build_choices, find_sessions};
    use std::fs;
    use std::path::Path;

    #[test]
    fn find_sessions_returns_sorted_matches() {
        let home = tempfile::tempdir().unwrap();
        for name in ["z.kitty-session", "a.kitty-session", "notes.txt"] {
            fs::write(home.path().join(name), name).unwrap();
        }

        let got = find_sessions(home.path()).unwrap();
        let want = [
            home.path().join("a.kitty-session"),
            home.path().join("z.kitty-session"),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn build_choices_uses_home_relative_labels() {
        let home = tempfile::tempdir().unwrap();
        let paths = [
            home.path().join("alpha.kitty-session"),
            home.path().join("nested").join("beta.kitty-session"),
        ];
        let got = build_choices(home.path(), &paths);
        let want = vec![
            Choice {
                label: "~/alpha.kitty-session".into(),
                path: paths[0].clone(),
            },
            Choice {
                label: Path::new("~")
                    .join("nested")
                    .join("beta.kitty-session")
                    .display()
                    .to_string(),
                path: paths[1].clone(),
            },
        ];
        assert_eq!(got, want);
    }
}
