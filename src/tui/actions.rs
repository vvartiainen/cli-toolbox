use std::path::{Path, PathBuf};

use crate::agent::vm::{self, VmOptions};
use crate::azure::{self, AccountAction, Az, AzCli};
use crate::sandbox::project::{self, CreateOptions, Project, SshKey};
use crate::{Error, agent, aws, git, kitty, ssh};

use super::app::{Action, NewProject, Resource, Tailor, ToolId};

pub(super) struct Context {
    cwd: Result<PathBuf, String>,
}

pub(super) struct Outcome {
    pub title: String,
    pub body: String,
    pub failed: bool,
    /// A command for the parent shell, such as an export or `cd`.
    pub shell: Option<String>,
    /// A project was created; ask how to tailor its flake.
    pub project: Option<NewProject>,
}

/// What a successful action reports back.
struct Done {
    body: String,
    shell: Option<String>,
    /// The action ran, but an optional step such as init failed.
    incomplete: bool,
    project: Option<NewProject>,
}

impl Done {
    fn body(body: String) -> Self {
        Self {
            body,
            shell: None,
            incomplete: false,
            project: None,
        }
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
            ToolId::GitWorktrees => Ok(git::project_worktrees(
                self.working_dir()?,
                &git::worktrees_root()?,
            )?
            .into_iter()
            .map(Resource::Worktree)
            .collect()),
            ToolId::SandboxProjects => Ok(vm::statuses(&project::sandbox_dir()?)?
                .into_iter()
                .map(|status| Resource::Project(Box::new(status)))
                .collect()),
            ToolId::AzureSubscription => {
                let subscriptions = AzCli::new()?.subscriptions()?;
                if subscriptions.is_empty() {
                    return Err(Error::msg(
                        "no Azure subscriptions: log in with Azure > Account > Log in",
                    ));
                }
                Ok(subscriptions
                    .into_iter()
                    .map(Resource::AzureSubscription)
                    .collect())
            }
            ToolId::AzureAccount => Ok(AccountAction::ALL
                .into_iter()
                .map(Resource::AzureAccount)
                .collect()),
            ToolId::SandboxVms => Ok(vm::running_vms()
                .into_iter()
                .map(|vm| Resource::Vm(Box::new(vm)))
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
        let (title, mut body, shell, failed, project) = match result {
            Ok(done) if done.incomplete => (
                "Action completed with errors",
                done.body,
                done.shell,
                true,
                done.project,
            ),
            Ok(done) => (
                "Action completed",
                done.body,
                done.shell,
                false,
                done.project,
            ),
            Err(err) => ("Action failed", err.to_string(), None, true, None),
        };
        for bytes in [&stdout, &stderr] {
            if !bytes.is_empty() {
                body.push_str("\n\n");
                body.push_str(&String::from_utf8_lossy(bytes));
            }
        }
        Outcome {
            title: title.into(),
            body,
            failed,
            shell,
            project,
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
                        "Profile ready: {}\n\nRun this in your shell after exiting (bash/zsh):\n{export}\n\nTo apply it automatically, start the launcher with `tui --print-shell` (see docs/tui.md).\n\nThe launcher's parent shell has not been changed.",
                        profile.name
                    ),
                    shell: Some(export),
                    incomplete: false,
                    project: None,
                })
            }
            Action::AzureSubscription(subscription) => {
                let export = azure::use_subscription(&AzCli::new()?, subscription, stderr)?;
                Ok(Done {
                    body: format!(
                        "Subscription ready: {}\n\nRun this in your shell after exiting (bash/zsh):\n{export}\n\nTo apply it automatically, start the launcher with `tui --print-shell` (see docs/tui.md).\n\nThe az CLI default subscription has been changed; the launcher's parent shell has not.",
                        subscription.name
                    ),
                    shell: Some(export),
                    incomplete: false,
                    project: None,
                })
            }
            Action::AzureAccount(action) => {
                let az = AzCli::new()?;
                let shell_subscription = std::env::var("AZURE_SUBSCRIPTION_ID").ok();
                match action {
                    AccountAction::Status => Ok(Done::body(azure::status(
                        &az,
                        shell_subscription.as_deref(),
                    ))),
                    AccountAction::Login | AccountAction::Relogin => {
                        let tenant = azure::tenant_from_env(None);
                        let force = *action == AccountAction::Relogin;
                        Ok(Done::body(azure::login(
                            &az,
                            tenant.as_deref(),
                            force,
                            stderr,
                        )?))
                    }
                    AccountAction::Logout => {
                        let unset = azure::logout(&az, stderr)?;
                        Ok(Done {
                            body: format!(
                                "Logged out.\n\nRun this in your shell after exiting (bash/zsh):\n{unset}"
                            ),
                            shell: Some(unset),
                            incomplete: false,
                            project: None,
                        })
                    }
                }
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
            Action::CreateWorktree(launch) => {
                // The launcher has released the terminal, so git and the init
                // command stream their progress to it directly.
                let cwd = self.working_dir()?;
                let root = git::git_worktrees_root()?;
                let name = git::next_agent_worktree_name(cwd, &root)?;
                let created = agent::create_worktree(
                    cwd,
                    &root,
                    &name,
                    agent::Options::default(),
                    launch.as_ref(),
                    &mut std::io::stderr(),
                )?;
                let path = &created.worktree.path;
                Ok(Done {
                    body: format!(
                        "Created worktree: {}\nBranch: {}\n\n{}\n\n{}",
                        path.display(),
                        created.worktree.branch,
                        steps_text(&created.steps),
                        directory_instructions(path)
                    ),
                    shell: cd_command(path),
                    incomplete: created.steps.iter().any(agent::Step::failed),
                    project: None,
                })
            }
            Action::CreateProject => {
                let options = CreateOptions {
                    name: None,
                    repo: git::main_checkout(self.working_dir()?)?,
                    template: None,
                    source: None,
                    ssh_key: SshKey::Auto,
                    proxy: true,
                    backlog: true,
                };
                let (project, created) = vm::create(
                    &project::sandbox_dir()?,
                    &options,
                    None,
                    None,
                    &mut std::io::stderr(),
                )?;
                let summary = format!(
                    "Sandbox project {}: {}\nFirst worktree: {}",
                    project.name,
                    project.dir.display(),
                    created.worktree.path.display(),
                );
                Ok(Done {
                    body: format!("{summary}\n\n{}", steps_text(&created.steps)),
                    shell: None,
                    incomplete: created.steps.iter().any(agent::Step::failed),
                    project: Some(NewProject {
                        name: project.name,
                        summary,
                    }),
                })
            }
            Action::TailorProject(name, how) => {
                let project = load_project(name)?;
                // The agent or editor takes over the released terminal.
                let done = match how {
                    Tailor::Agent(kind) => {
                        project::run_setup_agent(&project, None, kind, &mut std::io::stderr())?;
                        format!(
                            "{kind} finished tailoring {}/flake.nix.",
                            project.dir.display()
                        )
                    }
                    Tailor::Editor => {
                        project::edit_flake(&project)?;
                        format!("Edited {}/flake.nix.", project.dir.display())
                    }
                };
                Ok(Done::body(format!(
                    "{done}\n\nNext, open the project and press u to start the VM. The first start builds the guest, which takes a while; `nix build` in the project directory does it ahead of time."
                )))
            }
            Action::StartProject(name) => {
                let project = load_project(name)?;
                // Nix and the VM stream progress to the released terminal.
                let steps = vm::start(&project, &vm_options(), &mut std::io::stderr())?;
                Ok(steps_done(format!("VM of {name}"), &steps))
            }
            Action::StopProject(name) => {
                let steps = vm::stop(&load_project(name)?)?;
                Ok(steps_done(format!("VM of {name}"), &steps))
            }
            Action::RestartProject(name) => {
                let project = load_project(name)?;
                let steps = vm::restart(&project, &vm_options(), &mut std::io::stderr())?;
                Ok(steps_done(format!("VM of {name}"), &steps))
            }
            Action::SetupProject(name) => {
                // The proxy setup may prompt for secrets in the released terminal.
                let steps = vm::setup(&load_project(name)?);
                Ok(steps_done(format!("Setup of {name}"), &steps))
            }
            Action::SyncProject(name) => {
                let synced = vm::sync(&load_project(name)?, None)?;
                vm::write_synced(&synced, stdout)?;
                Ok(Done::body(format!("Review branches for {name}")))
            }
            Action::ReviewProject(name, worktree) => {
                let project = load_project(name)?;
                let target = agent::review::open(
                    &project,
                    worktree.as_ref().map(|worktree| worktree.name.as_str()),
                    stderr,
                )?;
                Ok(Done::body(match target {
                    Some(target) => {
                        format!("Reviewed {} in {}", target.branch, target.path.display())
                    }
                    None => "Review cancelled".into(),
                }))
            }
            Action::SyncAllProjects => {
                let steps = vm::sync_all(&project::sandbox_dir()?, stdout)?;
                Ok(steps_done(
                    "Published review branches for running projects".into(),
                    &steps,
                ))
            }
            Action::RestartAllProjects => {
                let options = vm_options();
                let steps = vm::each_running(&project::sandbox_dir()?, |project| {
                    vm::restart(project, &options, &mut std::io::stderr())
                })?;
                Ok(steps_done("Restarted every running VM".into(), &steps))
            }
            Action::StopAllProjects => Ok(steps_done(
                "Stopped every running VM".into(),
                &vm::each_running(&project::sandbox_dir()?, vm::stop)?,
            )),
            Action::AddProjectWorktree(name, launch) => {
                let created = vm::add_worktree(
                    &load_project(name)?,
                    false,
                    launch.as_ref(),
                    &vm_options(),
                    &mut std::io::stderr(),
                )?;
                Ok(Done {
                    body: format!(
                        "Created worktree: {}\nBranch: {}\n\n{}",
                        created.worktree.path.display(),
                        created.worktree.branch,
                        steps_text(&created.steps)
                    ),
                    shell: None,
                    incomplete: created.steps.iter().any(agent::Step::failed),
                    project: None,
                })
            }
            Action::DoneProjectWorktree(name, worktree) => {
                let steps = vm::done(&load_project(name)?, worktree, &vm_options(), stderr)?;
                Ok(steps_done(
                    format!(
                        "Removed {}; branch {} kept for review",
                        worktree.path.display(),
                        worktree.branch
                    ),
                    &steps,
                ))
            }
            Action::StopVm(runtime_dir) => {
                let vm = vm::running_vms()
                    .into_iter()
                    .find(|vm| vm.runtime_dir == *runtime_dir)
                    .ok_or_else(|| Error::msg("the VM is no longer running"))?;
                Ok(steps_done(format!("VM {}", vm.name), &vm::vm_stop(&vm)))
            }
            Action::PruneVms => Ok(steps_done(
                "Stale VM runtime directories".into(),
                &vm::vm_prune(&project::sandbox_dir()?)?,
            )),
            Action::EnterWorktree(worktree) | Action::EnterProjectWorktree(worktree) => Ok(Done {
                body: format!(
                    "Worktree: {}\n\n{}",
                    worktree.path.display(),
                    directory_instructions(&worktree.path)
                ),
                shell: cd_command(&worktree.path),
                incomplete: false,
                project: None,
            }),
            Action::Cleanup(worktree) => {
                let steps = agent::remove_worktree(
                    self.working_dir()?,
                    worktree,
                    false,
                    agent::Options::default(),
                    stderr,
                )?;
                let removed = if worktree.branch.is_empty() {
                    format!("Removed worktree {}", worktree.path.display())
                } else {
                    format!(
                        "Removed worktree {} and branch {}",
                        worktree.path.display(),
                        worktree.branch
                    )
                };
                Ok(Done {
                    body: format!("{removed}\n\n{}", steps_text(&steps)),
                    shell: None,
                    incomplete: steps.iter().any(agent::Step::failed),
                    project: None,
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

/// The coding agent that tailors new flakes: `CLI_TOOLBOX_AGENT`, or claude.
pub(super) fn setup_agent() -> String {
    default_agent().unwrap_or_else(|| "claude".into())
}

/// The agent new worktrees start with unless changed: `CLI_TOOLBOX_AGENT`.
pub(super) fn default_agent() -> Option<String> {
    vm_options().agent
}

/// VM options for the launcher: herdr on, the agent from `CLI_TOOLBOX_AGENT`.
fn vm_options() -> VmOptions {
    VmOptions {
        agent: std::env::var("CLI_TOOLBOX_AGENT")
            .ok()
            .filter(|kind| !kind.is_empty()),
        herdr: true,
        ..VmOptions::default()
    }
}

fn load_project(name: &str) -> Result<Project, Error> {
    project::load(&project::sandbox_dir()?, name)
}

fn steps_done(title: String, steps: &[agent::Step]) -> Done {
    Done {
        body: format!("{title}\n\n{}", steps_text(steps)),
        shell: None,
        incomplete: steps.iter().any(agent::Step::failed),
        project: None,
    }
}

fn steps_text(steps: &[agent::Step]) -> String {
    let lines: Vec<String> = steps.iter().map(ToString::to_string).collect();
    lines.join("\n")
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
        let outcome = context.execute(&Action::EnterWorktree(git::Worktree {
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
        let outcome = context.execute(&Action::CreateWorktree(None));
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
