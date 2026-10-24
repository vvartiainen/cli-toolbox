mod actions;
mod app;
mod ui;

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crossterm::event::{self, Event};

use crate::{Error, terminal::TerminalSession};
use actions::{Context, Outcome};
use app::{App, Effect, Prepare};

/// Run the launcher. With `print_shell`, a successful shell action (an AWS
/// export or a `cd`) exits the launcher and is printed to `stdout` for `eval`.
pub(crate) fn run(
    print_shell: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    check_terminals(
        io::stdin().is_terminal(),
        io::stdout().is_terminal(),
        io::stderr().is_terminal(),
        print_shell,
    )?;
    let context = Context::new();
    let mut app = App::new(context.cwd_display());
    app.print_shell = print_shell;
    app.herdr = crate::herdr::available();
    if app.herdr {
        app.agent_kinds = crate::herdr::agent_kinds();
    }
    app.default_agent = actions::default_agent();
    let mut session = Some(TerminalSession::inline(ui::HEIGHT)?);
    let result = event_loop(&context, &mut app, &mut session, stderr);
    let restoration = session.as_mut().map(Session::restore).transpose();
    match (result, restoration) {
        (Ok(shell), Ok(_)) => {
            if let Some(command) = shell {
                writeln!(stdout, "{command}")?;
            }
            Ok(())
        }
        (Err(err), Ok(_)) | (Ok(_), Err(err)) => Err(err),
        (Err(err), Err(cleanup)) => {
            Err(Error::msg(format!("{err}; cleanup also failed: {cleanup}")))
        }
    }
}

fn event_loop(
    context: &Context,
    app: &mut App,
    session: &mut Option<TerminalSession>,
    stderr: &mut dyn Write,
) -> Result<Option<String>, Error> {
    let print_shell = app.print_shell;
    loop {
        session
            .as_mut()
            .expect("active terminal")
            .terminal_mut()
            .draw(|frame| {
                ui::update_scroll_bounds(app, frame.area());
                ui::draw(frame, app);
            })?;
        if !event::poll(Duration::from_millis(200))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        match app.handle_key(key) {
            Effect::None => {}
            Effect::Quit => return Ok(None),
            Effect::Load(tool) => {
                session
                    .as_mut()
                    .expect("active terminal")
                    .terminal_mut()
                    .draw(|frame| ui::draw(frame, app))?;
                app.loaded(tool, context.load(tool).map_err(|err| err.to_string()));
            }
            Effect::Prepare(Prepare::ProjectWorktree(project)) => {
                let defaults = actions::project_worktree_defaults(&project);
                app.project_worktree_form(project, defaults.name, defaults.agents, defaults.note);
            }
            Effect::Prepare(Prepare::Project) => {
                app.project_form(context.project_defaults().map_err(|err| err.to_string()));
            }
            Effect::Execute(action) => {
                let outcome = handoff(
                    session,
                    || context.execute(&action),
                    |outcome| {
                        // Exiting with a shell command needs no launcher to return to.
                        if shell_exit(print_shell, outcome).is_some() {
                            return Ok(None);
                        }
                        TerminalSession::inline(ui::HEIGHT)
                            .map(Some)
                            .map_err(Error::from)
                    },
                    stderr,
                )?;
                if let Some(command) = shell_exit(print_shell, &outcome) {
                    return Ok(Some(command));
                }
                match outcome.project {
                    Some(project) if !outcome.failed => app.tailor_prompt(
                        project,
                        actions::setup_agent(),
                        crate::sandbox::project::editor().join(" "),
                    ),
                    _ => app.finished(&action, outcome.title, outcome.body, outcome.failed),
                }
            }
        }
    }
}

/// The launcher draws on stderr. Stdout may be redirected only when it
/// receives a shell command for `eval`.
fn check_terminals(
    stdin: bool,
    stdout: bool,
    stderr: bool,
    print_shell: bool,
) -> Result<(), Error> {
    if print_shell && !(stdin && stderr) {
        return Err(Error::msg(
            "tui --print-shell requires stdin and stderr to be terminals",
        ));
    }
    if print_shell || (stdin && stdout && stderr) {
        Ok(())
    } else {
        Err(Error::msg(
            "tui requires stdin, stdout, and stderr to be terminals; use --print-shell to capture shell commands, or a direct CLI command for redirected output",
        ))
    }
}

/// The command to print and exit with after an action, if any.
fn shell_exit(print_shell: bool, outcome: &Outcome) -> Option<String> {
    if !print_shell || outcome.failed {
        return None;
    }
    outcome.shell.clone()
}

trait Session {
    fn restore(&mut self) -> Result<(), Error>;
}

impl Session for TerminalSession {
    fn restore(&mut self) -> Result<(), Error> {
        TerminalSession::restore(self).map_err(Error::from)
    }
}

/// Give the terminal to an action, then resume the launcher. `resume` may
/// return `None` to leave the terminal released when the launcher exits next.
fn handoff<S: Session>(
    session: &mut Option<S>,
    execute: impl FnOnce() -> Outcome,
    resume: impl FnOnce(&Outcome) -> Result<Option<S>, Error>,
    stderr: &mut dyn Write,
) -> Result<Outcome, Error> {
    session.as_mut().expect("active terminal").restore()?;
    drop(session.take());
    let outcome = execute();
    match resume(&outcome) {
        Ok(next) => *session = next,
        Err(err) => {
            writeln!(stderr, "{}\n{}", outcome.title, outcome.body)?;
            return Err(Error::msg(format!("resume tool launcher: {err}")));
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct FakeSession {
        events: Rc<RefCell<Vec<&'static str>>>,
        fail_restore: bool,
    }

    impl Session for FakeSession {
        fn restore(&mut self) -> Result<(), Error> {
            self.events.borrow_mut().push("restore");
            if self.fail_restore {
                Err(Error::msg("restore failed"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn action_runs_between_terminal_sessions_even_when_action_fails() {
        for failed in [false, true] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let mut session = Some(FakeSession {
                events: events.clone(),
                fail_restore: false,
            });
            let mut stderr = Vec::new();
            let outcome = handoff(
                &mut session,
                || {
                    events.borrow_mut().push("execute");
                    Outcome {
                        title: "Result".into(),
                        body: "details".into(),
                        failed,
                        shell: None,
                        project: None,
                    }
                },
                |_| {
                    events.borrow_mut().push("initialize");
                    Ok(Some(FakeSession {
                        events: events.clone(),
                        fail_restore: false,
                    }))
                },
                &mut stderr,
            )
            .unwrap();
            assert_eq!(*events.borrow(), ["restore", "execute", "initialize"]);
            assert_eq!(outcome.failed, failed);
            assert!(stderr.is_empty());
        }
    }

    #[test]
    fn failed_restore_prevents_execution() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut session = Some(FakeSession {
            events: events.clone(),
            fail_restore: true,
        });
        assert!(
            handoff(
                &mut session,
                || panic!("must not execute"),
                |_| panic!("must not resume"),
                &mut Vec::new()
            )
            .is_err()
        );
        assert_eq!(*events.borrow(), ["restore"]);
    }

    #[test]
    fn failed_resume_preserves_completed_result_without_retry() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut session = Some(FakeSession {
            events: events.clone(),
            fail_restore: false,
        });
        let mut stderr = Vec::new();
        let result = handoff(
            &mut session,
            || {
                events.borrow_mut().push("execute");
                Outcome {
                    title: "Created".into(),
                    body: "/tmp/created-worktree".into(),
                    failed: false,
                    shell: None,
                    project: None,
                }
            },
            |_| Err(Error::msg("resume failed")),
            &mut stderr,
        );
        assert!(result.is_err());
        assert_eq!(*events.borrow(), ["restore", "execute"]);
        assert!(
            String::from_utf8(stderr)
                .unwrap()
                .contains("/tmp/created-worktree")
        );
    }

    #[test]
    fn requires_all_three_terminal_streams() {
        assert!(check_terminals(true, true, true, false).is_ok());
        for streams in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(check_terminals(streams.0, streams.1, streams.2, false).is_err());
        }
    }

    #[test]
    fn print_shell_allows_captured_stdout_only() {
        assert!(check_terminals(true, false, true, true).is_ok());
        assert!(check_terminals(false, false, true, true).is_err());
        assert!(check_terminals(true, false, false, true).is_err());
    }

    #[test]
    fn shell_exit_only_for_successful_shell_actions_in_print_mode() {
        let outcome = |failed, shell: Option<&str>| Outcome {
            title: String::new(),
            body: String::new(),
            failed,
            shell: shell.map(String::from),
            project: None,
        };
        assert_eq!(
            shell_exit(true, &outcome(false, Some("cd '/tmp/x'"))).as_deref(),
            Some("cd '/tmp/x'")
        );
        assert_eq!(
            shell_exit(false, &outcome(false, Some("cd '/tmp/x'"))),
            None
        );
        assert_eq!(shell_exit(true, &outcome(true, Some("cd '/tmp/x'"))), None);
        assert_eq!(shell_exit(true, &outcome(false, None)), None);
    }

    #[test]
    fn resume_can_leave_the_terminal_released() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut session = Some(FakeSession {
            events: events.clone(),
            fail_restore: false,
        });
        handoff(
            &mut session,
            || Outcome {
                title: String::new(),
                body: String::new(),
                failed: false,
                shell: Some("cd /tmp".into()),
                project: None,
            },
            |_| Ok(None),
            &mut Vec::new(),
        )
        .unwrap();
        assert!(session.is_none());
        assert_eq!(*events.borrow(), ["restore"]);
    }
}
