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
}

impl Context {
    pub fn new() -> Self {
        Self {
            cwd: std::env::current_dir().map_err(|err| format!("resolve working directory: {err}")),
        }
    }

    pub fn cwd_display(&self) -> String {
        match &self.cwd {
            Ok(path) => path.display().to_string(),
            Err(err) => err.clone(),
        }
    }

    fn working_dir(&self) -> Result<&Path, Error> {
        self.cwd.as_deref().map_err(Error::msg)
    }

    pub fn load(&self, tool: ToolId) -> Result<Vec<Resource>, Error> {
        match tool {
            ToolId::GitWorktreeList | ToolId::GitWorktreeCleanup => {
                Ok(git::agent_worktrees(self.working_dir()?)?
                    .into_iter()
                    .map(Resource::Worktree)
                    .collect())
            }
            ToolId::GitWorktreeCreate => Ok(Vec::new()),
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
        let mut body = match result {
            Ok(body) => body,
            Err(err) => err.to_string(),
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
        }
    }

    fn execute_inner(
        &self,
        action: &Action,
        stdout: &mut Vec<u8>,
        stderr: &mut Vec<u8>,
    ) -> Result<String, Error> {
        match action {
            Action::Aws(profile) => {
                let export = aws::export_profile(profile, stderr)?;
                Ok(format!(
                    "Profile ready: {}\n\nRun this in your shell after exiting (bash/zsh):\n{export}\n\nFor direct shell selection, use:\neval \"$(cli-toolbox aws profile)\"\n\nThe launcher's parent shell has not been changed.",
                    profile.name
                ))
            }
            Action::Kitty(path) => {
                kitty::launch_session(path, stdout, stderr)?;
                Ok(format!("Switched Kitty session to {}", path.display()))
            }
            Action::Ssh(host) => {
                ssh::connect(host, stdout, stderr)?;
                Ok(format!("SSH session for {} ended.", host.name))
            }
            Action::CreateWorktree => {
                let worktree = git::create_agent_worktree_record(self.working_dir()?, stderr)?;
                Ok(format!(
                    "Created worktree: {}\nBranch: {}\n\n{}",
                    worktree.path.display(),
                    worktree.branch,
                    directory_instructions(&worktree.path)
                ))
            }
            Action::Cleanup(worktree) => {
                let target = worktree.path.to_str().ok_or_else(|| {
                    Error::msg("cannot clean up a worktree whose path is not valid UTF-8")
                })?;
                git::cleanup_agent_worktree(self.working_dir()?, target, false, stdout, stderr)?;
                Ok(if worktree.branch.is_empty() {
                    format!("Removed worktree {}", worktree.path.display())
                } else {
                    format!(
                        "Removed worktree {} and branch {}",
                        worktree.path.display(),
                        worktree.branch
                    )
                })
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

fn directory_instructions(path: &Path) -> String {
    match path.to_str() {
        Some(path) => format!(
            "Run this in your shell after exiting (bash/zsh):\ncd {}\n\nThe launcher's parent directory has not been changed.",
            aws::shell_quote(path)
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
    }
}
