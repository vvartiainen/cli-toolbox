use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use crate::{Error, exec};

/// Prefer `fzf` when it is on PATH; otherwise use the built-in picker.
///
/// `fzf_height` is only used for `fzf` (for example `"~100%"` or `"40%"`).
pub fn pick(prompt: &str, items: &[String], fzf_height: &str) -> Result<Option<usize>, Error> {
    if items.is_empty() {
        return Ok(None);
    }
    if exec::look_path("fzf").is_some() {
        return pick_with_fzf(prompt, items, fzf_height);
    }
    pick_with_ratatui(prompt, items)
}

fn pick_with_fzf(prompt: &str, items: &[String], height: &str) -> Result<Option<usize>, Error> {
    let mut child = Command::new("fzf")
        .args(["--prompt", prompt, "--height", height, "--reverse"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|err| Error::msg(format!("select with fzf: {err}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(items.join("\n").as_bytes())
            .map_err(|err| Error::msg(format!("select with fzf: {err}")))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|err| Error::msg(format!("select with fzf: {err}")))?;

    match output.status.code() {
        Some(0) => index_of_selection(items, std::str::from_utf8(&output.stdout).unwrap_or("")),
        // 1 = no match / empty abort; 130 = Ctrl-C or Esc
        Some(1 | 130) => Ok(None),
        _ => Err(Error::msg(format!("select with fzf: {}", output.status))),
    }
}

fn pick_with_ratatui(prompt: &str, items: &[String]) -> Result<Option<usize>, Error> {
    if !io::stderr().is_terminal() {
        return Err(Error::msg("interactive selection requires a terminal"));
    }

    let mut stderr = io::stderr();
    enable_raw_mode()?;
    execute!(stderr, EnterAlternateScreen, Hide)?;
    let mut guard = TerminalGuard { restored: false };

    let backend = CrosstermBackend::new(io::stderr());
    let mut terminal = Terminal::new(backend)?;

    let mut query = String::new();
    let mut selected = 0usize;
    let result = loop {
        let visible = filter_items(&query, items);
        if selected >= visible.len() {
            selected = visible.len().saturating_sub(1);
        }

        terminal.draw(|frame| {
            draw(frame, prompt, &query, items, &visible, selected);
        })?;

        if !event::poll(Duration::from_millis(200))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            continue;
        }

        match handle_key(key, &mut query, &mut selected, visible.len()) {
            KeyAction::Continue => {}
            KeyAction::Confirm => {
                break Ok(visible.get(selected).copied());
            }
            KeyAction::Cancel => break Ok(None),
        }
    };

    guard.restore()?;
    result
}

pub(crate) fn filter_items(query: &str, items: &[String]) -> Vec<usize> {
    if query.is_empty() {
        return (0..items.len()).collect();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            pattern
                .score(Utf32Str::new(item, &mut buf), &mut matcher)
                .map(|score| (score, index))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

fn index_of_selection(items: &[String], selected: &str) -> Result<Option<usize>, Error> {
    let selected = selected.trim();
    if selected.is_empty() {
        return Ok(None);
    }
    items
        .iter()
        .position(|item| item == selected)
        .map(Some)
        .ok_or_else(|| Error::msg(format!("selected item {selected:?} was not found")))
}

enum KeyAction {
    Continue,
    Confirm,
    Cancel,
}

fn handle_key(
    key: KeyEvent,
    query: &mut String,
    selected: &mut usize,
    visible_len: usize,
) -> KeyAction {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c' | 'g') => return KeyAction::Cancel,
            KeyCode::Char('n') => move_selection(selected, visible_len, 1),
            KeyCode::Char('p') => move_selection(selected, visible_len, -1),
            KeyCode::Char('u') => query.clear(),
            _ => {}
        }
        return KeyAction::Continue;
    }

    match key.code {
        KeyCode::Esc => KeyAction::Cancel,
        KeyCode::Enter => {
            if visible_len == 0 {
                KeyAction::Continue
            } else {
                KeyAction::Confirm
            }
        }
        KeyCode::Up => {
            move_selection(selected, visible_len, -1);
            KeyAction::Continue
        }
        KeyCode::Down => {
            move_selection(selected, visible_len, 1);
            KeyAction::Continue
        }
        KeyCode::Backspace => {
            query.pop();
            *selected = 0;
            KeyAction::Continue
        }
        KeyCode::Char(ch) => {
            query.push(ch);
            *selected = 0;
            KeyAction::Continue
        }
        _ => KeyAction::Continue,
    }
}

fn move_selection(selected: &mut usize, visible_len: usize, delta: i32) {
    if visible_len == 0 {
        *selected = 0;
        return;
    }
    let next = *selected as i32 + delta;
    *selected = next.clamp(0, visible_len as i32 - 1) as usize;
}

fn draw(
    frame: &mut ratatui::Frame,
    prompt: &str,
    query: &str,
    items: &[String],
    visible: &[usize],
    selected: usize,
) {
    let layout = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]);
    let [list_area, input_area] = layout.areas(frame.area());

    let list_items: Vec<ListItem> = visible
        .iter()
        .map(|&index| ListItem::new(items[index].as_str()))
        .collect();
    let mut state = ListState::default();
    if !visible.is_empty() {
        state.select(Some(selected));
    }

    let list = List::new(list_items)
        .block(Block::bordered().title(prompt.trim_end()))
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list, list_area, &mut state);
    frame.render_widget(Paragraph::new(format!("{prompt}{query}")), input_area);
}

struct TerminalGuard {
    restored: bool,
}

impl TerminalGuard {
    fn restore(&mut self) -> io::Result<()> {
        if self.restored {
            return Ok(());
        }
        self.restored = true;
        disable_raw_mode()?;
        execute!(io::stderr(), LeaveAlternateScreen, Show)?;
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

#[cfg(test)]
mod tests {
    use super::{filter_items, index_of_selection};

    #[test]
    fn empty_query_keeps_original_order() {
        let items = vec!["prod".into(), "sandbox".into(), "dev".into()];
        assert_eq!(filter_items("", &items), vec![0, 1, 2]);
    }

    #[test]
    fn query_filters_and_ranks_matches() {
        let items = vec!["prod".into(), "sandbox".into(), "dev".into()];
        let matched = filter_items("sand", &items);
        assert_eq!(matched, vec![1]);
    }

    #[test]
    fn index_of_selection_maps_fzf_stdout() {
        let items = vec!["prod".into(), "sandbox".into(), "dev".into()];
        assert_eq!(index_of_selection(&items, "sandbox\n").unwrap(), Some(1));
        assert_eq!(index_of_selection(&items, "  \n").unwrap(), None);
        assert!(index_of_selection(&items, "missing").is_err());
    }
}
