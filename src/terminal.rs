use std::io;
use std::panic::{self, AssertUnwindSafe, PanicHookInfo, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread;

use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// Owns the terminal modes used by an interactive ratatui session.
///
/// The session takes ownership before making its first terminal mutation. This
/// means that a failed escape-sequence write is treated as if it might have
/// succeeded and is followed by a best-effort cleanup attempt.
pub(crate) struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stderr>>,
    state: Arc<Mutex<SessionState>>,
    panic_hook: Option<PanicHookGuard>,
    /// An inline viewport still drawn below the shell prompt.
    inline_pending: bool,
}

impl TerminalSession {
    pub(crate) fn new() -> io::Result<Self> {
        Self::from_parts(
            Terminal::new(CrosstermBackend::new(io::stderr()))?,
            Box::new(CrosstermOps {
                alternate_screen: true,
            }),
        )
    }

    /// Draw in a fixed-height area below the cursor instead of taking over
    /// the whole screen. The area is cleared again on restore.
    pub(crate) fn inline(height: u16) -> io::Result<Self> {
        let terminal = Terminal::with_options(
            CrosstermBackend::new(io::stderr()),
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Inline(height),
            },
        )?;
        let mut session = Self::from_parts(
            terminal,
            Box::new(CrosstermOps {
                alternate_screen: false,
            }),
        )?;
        session.inline_pending = true;
        Ok(session)
    }

    pub(crate) fn restore(&mut self) -> io::Result<()> {
        if self.inline_pending {
            self.inline_pending = false;
            // Leave the cursor where the launcher started so that the shell
            // prompt or a launched command continues from there.
            let origin = self.terminal.get_frame().area().as_position();
            let _ = self
                .terminal
                .clear()
                .and_then(|()| self.terminal.set_cursor_position(origin));
        }
        let result = lock_unpoisoned(&self.state).restore();
        if result.is_ok()
            && let Some(hook) = self.panic_hook.as_mut()
        {
            hook.disarm();
        }
        result
    }

    pub(crate) fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stderr>> {
        &mut self.terminal
    }

    fn from_parts(
        terminal: Terminal<CrosstermBackend<io::Stderr>>,
        operations: Box<dyn TerminalOps>,
    ) -> io::Result<Self> {
        Self::from_parts_inner(terminal, operations, true)
    }

    #[cfg(test)]
    fn from_parts_without_hook(
        terminal: Terminal<CrosstermBackend<io::Stderr>>,
        operations: Box<dyn TerminalOps>,
    ) -> io::Result<Self> {
        Self::from_parts_inner(terminal, operations, false)
    }

    fn from_parts_inner(
        terminal: Terminal<CrosstermBackend<io::Stderr>>,
        operations: Box<dyn TerminalOps>,
        install_panic_hook: bool,
    ) -> io::Result<Self> {
        let state = Arc::new(Mutex::new(SessionState::new(operations)));
        let panic_hook = install_panic_hook.then(|| PanicHookGuard::install(Arc::clone(&state)));

        let initialization = lock_unpoisoned(&state).initialize();
        if let Err(error) = initialization {
            let cleanup = lock_unpoisoned(&state).restore();
            if let Some(mut panic_hook) = panic_hook {
                panic_hook.disarm();
            }
            return Err(combine_errors(
                error,
                cleanup,
                "terminal initialization failed",
            ));
        }

        Ok(Self {
            terminal,
            state,
            panic_hook,
            inline_pending: false,
        })
    }

    #[cfg(test)]
    fn with_operations(operations: Box<dyn TerminalOps>) -> io::Result<Self> {
        Self::from_parts_without_hook(
            Terminal::with_options(
                CrosstermBackend::new(io::stderr()),
                ratatui::TerminalOptions {
                    viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 80, 24)),
                },
            )?,
            operations,
        )
    }

    #[cfg(test)]
    fn with_operations_and_hook(operations: Box<dyn TerminalOps>) -> io::Result<Self> {
        Self::from_parts(
            Terminal::with_options(
                CrosstermBackend::new(io::stderr()),
                ratatui::TerminalOptions {
                    viewport: ratatui::Viewport::Fixed(ratatui::layout::Rect::new(0, 0, 80, 24)),
                },
            )?,
            operations,
        )
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.restore();

        // If cleanup itself failed, restore the process-wide hook anyway. A
        // dropped session cannot report that error, and retaining a hook that
        // points at dead session state would make later panics surprising.
        if let Some(hook) = self.panic_hook.as_mut() {
            hook.disarm();
        }
    }
}

trait TerminalOps: Send {
    fn enable_raw_mode(&mut self) -> io::Result<()>;
    fn disable_raw_mode(&mut self) -> io::Result<()>;
    fn enter_alternate_screen(&mut self) -> io::Result<()>;
    fn leave_alternate_screen(&mut self) -> io::Result<()>;
    fn hide_cursor(&mut self) -> io::Result<()>;
    fn show_cursor(&mut self) -> io::Result<()>;
}

struct CrosstermOps {
    alternate_screen: bool,
}

impl TerminalOps for CrosstermOps {
    fn enable_raw_mode(&mut self) -> io::Result<()> {
        enable_raw_mode()
    }

    fn disable_raw_mode(&mut self) -> io::Result<()> {
        disable_raw_mode()
    }

    fn enter_alternate_screen(&mut self) -> io::Result<()> {
        if self.alternate_screen {
            execute!(io::stderr(), EnterAlternateScreen)?;
        }
        Ok(())
    }

    fn leave_alternate_screen(&mut self) -> io::Result<()> {
        if self.alternate_screen {
            execute!(io::stderr(), LeaveAlternateScreen)?;
        }
        Ok(())
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        execute!(io::stderr(), Hide)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        execute!(io::stderr(), Show)
    }
}

struct SessionState {
    operations: Box<dyn TerminalOps>,
    raw_mode_pending: bool,
    alternate_screen_pending: bool,
    cursor_hidden_pending: bool,
}

impl SessionState {
    fn new(operations: Box<dyn TerminalOps>) -> Self {
        Self {
            operations,
            raw_mode_pending: false,
            alternate_screen_pending: false,
            cursor_hidden_pending: false,
        }
    }

    fn initialize(&mut self) -> io::Result<()> {
        // Set each flag before the operation. Crossterm can have written part
        // of an escape sequence when an operation returns an error.
        self.raw_mode_pending = true;
        if let Err(error) = self.operations.enable_raw_mode() {
            let cleanup = self.restore();
            return Err(combine_errors(error, cleanup, "enabling raw mode failed"));
        }

        self.alternate_screen_pending = true;
        if let Err(error) = self.operations.enter_alternate_screen() {
            let cleanup = self.restore();
            return Err(combine_errors(
                error,
                cleanup,
                "entering the alternate screen failed",
            ));
        }

        self.cursor_hidden_pending = true;
        if let Err(error) = self.operations.hide_cursor() {
            let cleanup = self.restore();
            return Err(combine_errors(error, cleanup, "hiding the cursor failed"));
        }

        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        let mut errors = Vec::new();

        if self.raw_mode_pending {
            match self.operations.disable_raw_mode() {
                Ok(()) => self.raw_mode_pending = false,
                Err(error) => errors.push(("disabling raw mode", error)),
            }
        }

        if self.alternate_screen_pending {
            match self.operations.leave_alternate_screen() {
                Ok(()) => self.alternate_screen_pending = false,
                Err(error) => errors.push(("leaving the alternate screen", error)),
            }
        }

        if self.cursor_hidden_pending {
            match self.operations.show_cursor() {
                Ok(()) => self.cursor_hidden_pending = false,
                Err(error) => errors.push(("showing the cursor", error)),
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            let details = errors
                .into_iter()
                .map(|(operation, error)| format!("{operation}: {error}"))
                .collect::<Vec<_>>()
                .join("; ");
            Err(io::Error::other(format!(
                "terminal restoration failed: {details}"
            )))
        }
    }
}

type PanicHook = dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static;

struct PanicHookGuard {
    previous: Arc<Mutex<Option<Box<PanicHook>>>>,
    active: bool,
}

impl PanicHookGuard {
    fn install(state: Arc<Mutex<SessionState>>) -> Self {
        let previous = Arc::new(Mutex::new(Some(panic::take_hook())));
        let previous_for_hook = Arc::clone(&previous);

        panic::set_hook(Box::new(move |panic_info| {
            // A panic must not leave the process in raw mode before the
            // previous hook prints its diagnostic. A broken terminal operation
            // cannot be reported safely from a panic hook, so ignore that
            // result and always delegate to the previous hook.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                restore_from_panic(&state);
            }));

            let previous = lock_unpoisoned(&previous_for_hook);
            if let Some(previous) = previous.as_ref() {
                previous(panic_info);
            }
        }));

        Self {
            previous,
            active: true,
        }
    }

    fn disarm(&mut self) {
        if !self.active {
            return;
        }

        // `set_hook` panics when called from a panicking thread. The hook has
        // already restored the terminal before diagnostics are delegated, and
        // the process will normally exit after an uncaught panic, so leave the
        // scoped hook in place for this last-resort path instead of causing a
        // second panic from Drop.
        if thread::panicking() {
            return;
        }
        self.active = false;

        if let Some(previous) = lock_unpoisoned(&self.previous).take() {
            panic::set_hook(previous);
        }
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn restore_from_panic(state: &Mutex<SessionState>) {
    match state.try_lock() {
        Ok(mut state) => {
            let _ = state.restore();
        }
        Err(TryLockError::Poisoned(poisoned)) => {
            let _ = poisoned.into_inner().restore();
        }
        // A panic from inside a terminal operation can happen while the
        // session mutex is held. Blocking in the panic hook would deadlock;
        // Drop will make its own best-effort attempt during unwinding.
        Err(TryLockError::WouldBlock) => {}
    }
}

fn combine_errors(primary: io::Error, cleanup: io::Result<()>, context: &str) -> io::Error {
    match cleanup {
        Ok(()) => io::Error::new(primary.kind(), format!("{context}: {primary}")),
        Err(cleanup) => io::Error::new(
            primary.kind(),
            format!("{context}: {primary}; cleanup also failed: {cleanup}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::env;
    use std::panic::{self, AssertUnwindSafe};
    use std::process::Command;
    use std::sync::{Arc, Mutex};

    use super::{TerminalOps, TerminalSession};

    #[derive(Clone)]
    struct FakeOps {
        calls: Arc<Mutex<Vec<&'static str>>>,
        failures: Arc<Mutex<HashSet<&'static str>>>,
    }

    impl FakeOps {
        fn new() -> (Self, Arc<Mutex<Vec<&'static str>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let failures = Arc::new(Mutex::new(HashSet::new()));
            (
                Self {
                    calls: Arc::clone(&calls),
                    failures,
                },
                calls,
            )
        }

        fn fail(&self, operation: &'static str) {
            self.failures.lock().unwrap().insert(operation);
        }

        fn call(&mut self, operation: &'static str) -> std::io::Result<()> {
            self.calls.lock().unwrap().push(operation);
            if self.failures.lock().unwrap().contains(operation) {
                Err(std::io::Error::other(operation))
            } else {
                Ok(())
            }
        }
    }

    impl TerminalOps for FakeOps {
        fn enable_raw_mode(&mut self) -> std::io::Result<()> {
            self.call("enable_raw_mode")
        }

        fn disable_raw_mode(&mut self) -> std::io::Result<()> {
            self.call("disable_raw_mode")
        }

        fn enter_alternate_screen(&mut self) -> std::io::Result<()> {
            self.call("enter_alternate_screen")
        }

        fn leave_alternate_screen(&mut self) -> std::io::Result<()> {
            self.call("leave_alternate_screen")
        }

        fn hide_cursor(&mut self) -> std::io::Result<()> {
            self.call("hide_cursor")
        }

        fn show_cursor(&mut self) -> std::io::Result<()> {
            self.call("show_cursor")
        }
    }

    fn calls(calls: &Arc<Mutex<Vec<&'static str>>>) -> Vec<&'static str> {
        calls.lock().unwrap().clone()
    }

    #[test]
    fn initializes_and_restores_in_order() {
        let (operations, call_log) = FakeOps::new();
        let mut session = TerminalSession::with_operations(Box::new(operations)).unwrap();

        session.restore().unwrap();

        assert_eq!(
            calls(&call_log),
            vec![
                "enable_raw_mode",
                "enter_alternate_screen",
                "hide_cursor",
                "disable_raw_mode",
                "leave_alternate_screen",
                "show_cursor",
            ]
        );
        session.restore().unwrap();
        assert_eq!(calls(&call_log).len(), 6);
    }

    #[test]
    fn failed_initialization_cleans_up_every_possible_stage() {
        for (failed_stage, expected_calls) in [
            (
                "enable_raw_mode",
                vec!["enable_raw_mode", "disable_raw_mode"],
            ),
            (
                "enter_alternate_screen",
                vec![
                    "enable_raw_mode",
                    "enter_alternate_screen",
                    "disable_raw_mode",
                    "leave_alternate_screen",
                ],
            ),
            (
                "hide_cursor",
                vec![
                    "enable_raw_mode",
                    "enter_alternate_screen",
                    "hide_cursor",
                    "disable_raw_mode",
                    "leave_alternate_screen",
                    "show_cursor",
                ],
            ),
        ] {
            let (operations, call_log) = FakeOps::new();
            operations.fail(failed_stage);

            let result = TerminalSession::with_operations(Box::new(operations));

            assert!(result.is_err());
            assert_eq!(calls(&call_log), expected_calls);
        }
    }

    #[test]
    fn restoration_attempts_independent_operations_and_retries_failures() {
        let (operations, call_log) = FakeOps::new();
        let operations_for_test = operations.clone();
        let mut session = TerminalSession::with_operations(Box::new(operations)).unwrap();
        operations_for_test.fail("disable_raw_mode");
        operations_for_test.fail("leave_alternate_screen");

        assert!(session.restore().is_err());
        assert_eq!(
            calls(&call_log)[3..],
            ["disable_raw_mode", "leave_alternate_screen", "show_cursor"]
        );

        operations_for_test.failures.lock().unwrap().clear();
        session.restore().unwrap();
        assert_eq!(
            calls(&call_log)[6..],
            ["disable_raw_mode", "leave_alternate_screen"]
        );
        session.restore().unwrap();
        assert_eq!(calls(&call_log).len(), 8);
    }

    #[test]
    fn panic_hook_restores_before_delegating_to_previous_hook() {
        if run_hook_test_in_child("panic_hook_restores_before_delegating_to_previous_hook") {
            return;
        }

        let previous = panic::take_hook();
        let (observed_ops, observed_calls) = FakeOps::new();
        let previous_saw_cleanup = Arc::new(Mutex::new(false));
        let previous_saw_cleanup_for_hook = Arc::clone(&previous_saw_cleanup);
        let observed_calls_for_hook = Arc::clone(&observed_calls);
        panic::set_hook(Box::new(move |_| {
            *previous_saw_cleanup_for_hook.lock().unwrap() =
                calls(&observed_calls_for_hook).contains(&"show_cursor");
        }));

        let mut session =
            TerminalSession::with_operations_and_hook(Box::new(observed_ops)).unwrap();
        let result = panic::catch_unwind(AssertUnwindSafe(|| panic!("test panic")));

        assert!(result.is_err());
        assert!(*previous_saw_cleanup.lock().unwrap());
        assert!(session.restore().is_ok());
        panic::set_hook(previous);
    }

    #[test]
    fn drop_during_unwind_does_not_trigger_a_second_panic() {
        if run_hook_test_in_child("drop_during_unwind_does_not_trigger_a_second_panic") {
            return;
        }

        let previous = panic::take_hook();
        panic::set_hook(Box::new(|_| {}));
        let (operations, call_log) = FakeOps::new();
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _session = TerminalSession::with_operations(Box::new(operations)).unwrap();
            panic!("test panic");
        }));

        // The session's hook is still installed while the thread is unwinding;
        // it cannot call set_hook from Drop. Restore the test process hook once
        // catch_unwind has returned, when doing so is legal again.
        let installed_hook = panic::take_hook();
        panic::set_hook(previous);
        drop(installed_hook);

        assert!(result.is_err());
        assert_eq!(
            calls(&call_log),
            vec![
                "enable_raw_mode",
                "enter_alternate_screen",
                "hide_cursor",
                "disable_raw_mode",
                "leave_alternate_screen",
                "show_cursor",
            ]
        );
    }

    const HOOK_TEST_CHILD_ENV: &str = "CLI_TOOLBOX_TERMINAL_HOOK_TEST_CHILD";

    fn run_hook_test_in_child(test_name: &str) -> bool {
        if env::var_os(HOOK_TEST_CHILD_ENV).is_some() {
            return false;
        }

        let filter = format!("terminal::tests::{test_name}");
        let status = Command::new(env::current_exe().expect("test executable path"))
            .args(["--exact", filter.as_str(), "--nocapture"])
            .env(HOOK_TEST_CHILD_ENV, "1")
            .status()
            .expect("run isolated panic-hook test");
        assert!(status.success(), "isolated test process failed: {status}");
        true
    }
}
