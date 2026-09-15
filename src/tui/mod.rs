mod actions;
mod app;
mod ui;

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use crossterm::event::{self, Event};

use crate::{Error, terminal::TerminalSession};
use actions::{Context, Outcome};
use app::{App, Effect};

pub(crate) fn run(_stdout: &mut dyn Write, stderr: &mut dyn Write) -> Result<(), Error> {
    check_terminals(
        io::stdin().is_terminal(),
        io::stdout().is_terminal(),
        io::stderr().is_terminal(),
    )?;
    let context = Context::new();
    let mut app = App::new(context.cwd_display());
    let mut session = Some(TerminalSession::new()?);
    let result = event_loop(&context, &mut app, &mut session, stderr);
    let restoration = session.as_mut().map(Session::restore).transpose();
    match (result, restoration) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(err), Ok(_)) | (Ok(()), Err(err)) => Err(err),
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
) -> Result<(), Error> {
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
            Effect::Quit => return Ok(()),
            Effect::Load(tool) => {
                session
                    .as_mut()
                    .expect("active terminal")
                    .terminal_mut()
                    .draw(|frame| ui::draw_loading(frame, app))?;
                app.loaded(tool, context.load(tool).map_err(|err| err.to_string()));
            }
            Effect::Execute(action) => {
                let outcome = handoff(
                    session,
                    || context.execute(&action),
                    || TerminalSession::new().map_err(Error::from),
                    stderr,
                )?;
                app.finished(&action, outcome.title, outcome.body, outcome.failed);
            }
        }
    }
}

fn check_terminals(stdin: bool, stdout: bool, stderr: bool) -> Result<(), Error> {
    if stdin && stdout && stderr {
        Ok(())
    } else {
        Err(Error::msg(
            "tui requires stdin, stdout, and stderr to be terminals; use a direct CLI command for redirected output",
        ))
    }
}

trait Session {
    fn restore(&mut self) -> Result<(), Error>;
}

impl Session for TerminalSession {
    fn restore(&mut self) -> Result<(), Error> {
        TerminalSession::restore(self).map_err(Error::from)
    }
}

fn handoff<S: Session>(
    session: &mut Option<S>,
    execute: impl FnOnce() -> Outcome,
    initialize: impl FnOnce() -> Result<S, Error>,
    stderr: &mut dyn Write,
) -> Result<Outcome, Error> {
    session.as_mut().expect("active terminal").restore()?;
    drop(session.take());
    let outcome = execute();
    match initialize() {
        Ok(active) => *session = Some(active),
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
                    }
                },
                || {
                    events.borrow_mut().push("initialize");
                    Ok(FakeSession {
                        events: events.clone(),
                        fail_restore: false,
                    })
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
                || panic!("must not resume"),
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
                }
            },
            || Err(Error::msg("resume failed")),
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
        assert!(check_terminals(true, true, true).is_ok());
        for streams in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(check_terminals(streams.0, streams.1, streams.2).is_err());
        }
    }
}
