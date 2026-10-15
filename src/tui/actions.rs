use std::path::{Path, PathBuf};

use crate::{Error, aws, git, kitty, ssh};

use super::app::{Action, Resource, ToolId};

pub(super) struct Context {
    cwd: Result<PathBuf, String>,
}

pub(super) struct Outcome {
    pub title: String,
    pub body: String,
    pub failed: bool,
    /// A command for the parent shell, such as an export or `cd`.
    pub shell: Option<String>,
}

/// What a successful action reports back.
struct Done {
    body: String,
    shell: Option<String>,
}

impl Done {
    fn body(body: String) -> Self {
        Self { body, shell: None }
    }
}

impl Context {
    pub fn new() -> Self {
        Self {
            cwd: std::env::current_dir().map_err(|err| format!("resolve working directory: {err}")),
        }
    }

    pub fn cwd_display(&self) -> String {
        match &self.cwd {
            Ok(path) => match dirs::home_dir().and_then(|home| path.strip_prefix(home).ok()) {
                Some(relative) if relative.as_os_str().is_empty() => "~".into(),
                Some(relative) => format!("~/{}", relative.display()),
                None => path.display().to_string(),
            },
            Err(err) => err.clone(),
        }
    }

    fn working_dir(&self) -> Result<&Path, Error> {
        self.cwd.as_deref().map_err(Error::msg)
    }

    pub fn load(&self, tool: ToolId) -> Result<Vec<Resource>, Error> {
        match tool {
            ToolId::AgentWorktrees => Ok(git::agent_worktrees(self.working_dir()?)?
                .into_iter()
                .map(Resource::Worktree)
                .collect()),
            _ => {
                let home = dirs::home_dir().ok_or_else(|| {
                    Error::msg("resolve home directory: home directory not found")
                })?;
                load_home(tool, &home)
            }
        }
    }

    pub fn execute(&self, action: &Action) -> Outcome {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = self.execute_inner(action, &mut stdout, &mut stderr);
        let failed = result.is_err();
        let (mut body, shell) = match result {
            Ok(done) => (done.body, done.shell),
            Err(err) => (err.to_string(), None),
        };
        for bytes in [&stdout, &stderr] {
            if !bytes.is_empty() {
                body.push_str("\n\n");
                body.push_str(&String::from_utf8_lossy(bytes));
            }
        }
        Outcome {
            title: if failed {
                "Action failed"
            } else {
                "Action completed"
            }
            .into(),
            body,
            failed,
            shell,
        }
    }

    fn execute_inner(
        &self,
        action: &Action,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> Result<Done, Error> {
        match action {
            Action::Aws(profile) => {
                let export = aws::export_profile(profile, stderr)?;
                Ok(Done {
                    body: format!(
                        "Profile ready: {}\n\nRun this in your shell after exiting (bash/zsh):\n{export}\n\nTo apply it automatically, start the launcher with `tui --print-shell` (see README).\n\nThe launcher's parent shell has not been changed.",
                        profile.name
                    ),
                    shell: Some(export),
                })
            }
            Action::Kitty(path) => {
                kitty::launch_session(path, stdout, stderr)?;
                Ok(Done::body(format!(
                    "Switched Kitty session to {}",
                    path.display()
                )))
            }
            Action::Ssh(host) => {
                ssh::connect(host, stdout, stderr)?;
                Ok(Done::body(format!("SSH session for {} ended.", host.name)))
            }
            Action::CreateWorktree => {
                let worktree = git::create_agent_worktree_record(self.working_dir()?, stderr)?;
                Ok(Done {
                    body: format!(
                        "Created worktree: {}\nBranch: {}\n\n{}",
                        worktree.path.display(),
                        worktree.branch,
                        directory_instructions(&worktree.path)
                    ),
                    shell: cd_command(&worktree.path),
                })
            }
            Action::EnterWorktree(worktree) => Ok(Done {
                body: format!(
                    "Worktree: {}\n\n{}",
                    worktree.path.display(),
                    directory_instructions(&worktree.path)
                ),
                shell: cd_command(&worktree.path),
            }),
            Action::Cleanup(worktree) => {
                let target = worktree.path.to_str().ok_or_else(|| {
                    Error::msg("cannot clean up a worktree whose path is not valid UTF-8")
                })?;
                git::cleanup_agent_worktree(self.working_dir()?, target, false, stdout, stderr)?;
                Ok(Done::body(if worktree.branch.is_empty() {
                    format!("Removed worktree {}", worktree.path.display())
                } else {
                    format!(
                        "Removed worktree {} and branch {}",
                        worktree.path.display(),
                        worktree.branch
                    )
                }))
            }
        }
    }
}

fn load_home(tool: ToolId, home: &Path) -> Result<Vec<Resource>, Error> {
    match tool {
        ToolId::AwsProfile => Ok(aws::load_profiles(
            &home.join(".aws/config"),
            &home.join(".aws/credentials"),
        )?
        .into_iter()
        .map(Resource::Aws)
        .collect()),
        ToolId::KittySession => {
            let paths = kitty::find_sessions(home)?;
            Ok(kitty::build_choices(home, &paths)
                .into_iter()
                .map(Resource::Kitty)
                .collect())
        }
        ToolId::SshConnect => Ok(ssh::load_hosts(&home.join(".ssh/config"))?
            .into_iter()
            .map(Resource::Ssh)
            .collect()),
        _ => Err(Error::msg(
            "this tool does not load resources from the home directory",
        )),
    }
}

/// A `cd` for bash/zsh, or nothing when the path cannot be quoted safely.
fn cd_command(path: &Path) -> Option<String> {
    path.to_str()
        .map(|path| format!("cd {}", aws::shell_quote(path)))
}

fn directory_instructions(path: &Path) -> String {
    match cd_command(path) {
        Some(cd) => format!(
            "Run this in your shell after exiting (bash/zsh):\n{cd}\n\nThe launcher's parent directory has not been changed."
        ),
        None => "This path is not valid UTF-8; no shell command can be displayed safely.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn selected_non_sso_profile_returns_shell_instructions() {
        let context = Context {
            cwd: Err("no working directory".into()),
        };
        let outcome = context.execute(&Action::Aws(aws::Profile {
            name: "dev's profile".into(),
            settings: BTreeMap::new(),
        }));
        assert!(!outcome.failed, "{}", outcome.body);
        assert!(
            outcome
                .body
                .contains("export AWS_PROFILE='dev'\\''s profile'")
        );
        assert!(outcome.body.contains("after exiting"));
        assert_eq!(
            outcome.shell.as_deref(),
            Some("export AWS_PROFILE='dev'\\''s profile'")
        );
    }

    #[test]
    fn entering_a_worktree_returns_a_quoted_cd() {
        let context = Context {
            cwd: Err("not needed".into()),
        };
        let outcome = context.execute(&Action::EnterWorktree(git::AgentWorktree {
            name: "agent's tree".into(),
            path: PathBuf::from("/tmp/agent's tree"),
            branch: String::new(),
        }));
        assert!(!outcome.failed, "{}", outcome.body);
        assert_eq!(outcome.shell.as_deref(), Some("cd '/tmp/agent'\\''s tree'"));
    }

    #[test]
    fn discovery_uses_fixture_home_and_preserves_session_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("work.kitty-session");
        std::fs::write(&path, "layout tall\n").unwrap();
        let resources = load_home(ToolId::KittySession, temp.path()).unwrap();
        assert!(matches!(&resources[..], [Resource::Kitty(choice)] if choice.path == path));
    }

    #[test]
    fn missing_working_directory_is_a_recoverable_git_error() {
        let context = Context {
            cwd: Err("directory no longer exists".into()),
        };
        let outcome = context.execute(&Action::CreateWorktree);
        assert!(outcome.failed);
        assert!(outcome.shell.is_none());
        assert!(outcome.body.contains("directory no longer exists"));
    }

    #[test]
    fn shell_directory_command_quotes_apostrophes() {
        let text = directory_instructions(Path::new("/tmp/agent's tree"));
        assert!(text.contains("cd '/tmp/agent'\\''s tree'"));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_path_does_not_generate_a_lossy_cd_command() {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/invalid-\xff".to_vec()));
        let text = directory_instructions(&path);
        assert!(!text.contains("cd "));
        assert!(text.contains("not valid UTF-8"));
        assert!(cd_command(&path).is_none());
    }
}
