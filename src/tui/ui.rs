use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, ConfirmState, DetailsState, PickerState, ResultState, Screen, ToolId};

const HELP_HOME: &str = "↑/↓ or Ctrl-P/Ctrl-N move • type to filter • Enter open • Esc/Ctrl-C quit";
const HELP_PICKER: &str = "↑/↓ move • type to filter • Enter select • F5 refresh • Esc back";
const HELP_RESULT: &str = "↑/↓ scroll • PageUp/PageDown page • Enter/Esc back";

/// Update viewport-dependent state before drawing, including after a resize.
pub(super) fn update_scroll_bounds(app: &mut App, area: Rect) {
    let inner = Block::bordered().inner(area);
    match &mut app.screen {
        Screen::Result(result) => {
            let [_, body, _] = result_layout(inner);
            result.scroll = result
                .scroll
                .min(max_scroll(&result_paragraph(result), body));
        }
        Screen::WorktreeDetails(details) => {
            let [body, _] = details_layout(inner);
            details.scroll = details
                .scroll
                .min(max_scroll(&details_paragraph(details), body));
        }
        _ => {}
    }
}

fn max_scroll(paragraph: &Paragraph<'_>, area: Rect) -> u16 {
    paragraph
        .line_count(area.width)
        .saturating_sub(usize::from(area.height))
        .min(usize::from(u16::MAX)) as u16
}

fn details_layout(inner: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner)
}

fn result_layout(inner: Rect) -> [Rect; 3] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner)
}

pub(crate) fn draw(frame: &mut Frame<'_>, app: &App) {
    match &app.screen {
        Screen::Home => draw_home(frame, app),
        Screen::Picker(picker) => draw_picker(frame, app, picker),
        Screen::WorktreeDetails(details) => draw_details(frame, details),
        Screen::ConfirmCleanup(confirm) => draw_confirmation(frame, confirm),
        Screen::Result(result) => draw_result(frame, result),
    }
}

/// Draw the short loading frame used immediately before synchronous discovery.
pub(crate) fn draw_loading(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let block = Block::bordered().title("cli-toolbox");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let tool = app.loading.map(ToolId::picker_title).unwrap_or("resources");
    let text = Paragraph::new(format!("Loading {tool}…"))
        .style(Style::default().fg(Color::Yellow))
        .wrap(Wrap { trim: true });
    frame.render_widget(text, inner);
}

fn draw_home(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let block = Block::bordered().title("cli-toolbox");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [cwd, search, list, description, help] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(inner);

    frame.render_widget(
        Paragraph::new(format!("Directory: {}", clean_text(&app.cwd_display))),
        cwd,
    );
    frame.render_widget(
        Paragraph::new(format!("Search: {}", clean_text(&app.home_query))),
        search,
    );

    let visible = app.home_visible_indices();
    let items: Vec<ListItem> = visible
        .iter()
        .filter_map(|index| ToolId::ALL.get(*index))
        .map(|tool| ListItem::new(tool.label()))
        .collect();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.home_selected.min(items.len() - 1)));
    }
    let list_widget = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Actions"))
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(list_widget, list, &mut state);

    let description_text = app
        .selected_tool()
        .map(ToolId::description)
        .unwrap_or_else(|| {
            if app.home_query.is_empty() {
                "No actions match the current selection."
            } else {
                "No actions match this search."
            }
        });
    frame.render_widget(
        Paragraph::new(clean_text(description_text)).wrap(Wrap { trim: true }),
        description,
    );
    frame.render_widget(
        Paragraph::new(HELP_HOME).style(Style::default().fg(Color::DarkGray)),
        help,
    );
}

fn draw_picker(frame: &mut Frame<'_>, app: &App, picker: &PickerState) {
    let area = frame.area();
    let block = Block::bordered().title(picker.tool.picker_title());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [search, list, message, help] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(format!("Search: {}", clean_text(&picker.query))),
        search,
    );

    let visible = picker.visible_indices();
    let items: Vec<ListItem> = visible
        .iter()
        .filter_map(|index| picker.resources.get(*index))
        .map(|resource| ListItem::new(clean_text(&resource.label())))
        .collect();
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(picker.selected.min(items.len() - 1)));
    }
    let widget = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Select"))
        .highlight_symbol("> ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    frame.render_stateful_widget(widget, list, &mut state);

    let message_text = if let Some(error) = &picker.error {
        format!("Error: {}", clean_text(error))
    } else if picker.resources.is_empty() {
        format!(
            "No {} found. Press F5 to refresh or Esc to go back.",
            picker.tool.picker_title().to_ascii_lowercase()
        )
    } else if visible.is_empty() {
        "No matches. Press Backspace or Ctrl-U to change the search.".to_string()
    } else {
        String::new()
    };
    frame.render_widget(
        Paragraph::new(message_text).style(Style::default().fg(Color::Yellow)),
        message,
    );
    frame.render_widget(
        Paragraph::new(HELP_PICKER).style(Style::default().fg(Color::DarkGray)),
        help,
    );

    if app.loading.is_some() {
        draw_loading_overlay(frame, area, picker.tool.picker_title());
    }
}

fn draw_loading_overlay(frame: &mut Frame<'_>, area: Rect, title: &str) {
    let popup = centered_rect(area, 60, 20);
    let block = Block::bordered().title(format!("Loading {title}"));
    let text = Paragraph::new("Reading resources…")
        .block(block)
        .style(Style::default().fg(Color::Yellow))
        .wrap(Wrap { trim: true });
    frame.render_widget(text, popup);
}

fn draw_details(frame: &mut Frame<'_>, details: &DetailsState) {
    let area = frame.area();
    let block = Block::bordered().title("Worktree details");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [body, help] = details_layout(inner);
    let paragraph = details_paragraph(details);
    let scroll = details.scroll.min(max_scroll(&paragraph, body));
    frame.render_widget(paragraph.scroll((scroll, 0)), body);
    frame.render_widget(
        Paragraph::new(HELP_RESULT).style(Style::default().fg(Color::DarkGray)),
        help,
    );
}

fn details_paragraph(details: &DetailsState) -> Paragraph<'static> {
    let branch = if details.worktree.branch.is_empty() {
        "(no branch)"
    } else {
        &details.worktree.branch
    };
    let content = vec![
        Line::from(vec![
            Span::styled("Name: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(clean_text(&details.worktree.name)),
        ]),
        Line::from(vec![
            Span::styled("Path: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(clean_text(&details.worktree.path.display().to_string())),
        ]),
        Line::from(vec![
            Span::styled("Branch: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(clean_text(branch)),
        ]),
    ];
    Paragraph::new(Text::from(content)).wrap(Wrap { trim: true })
}

fn draw_confirmation(frame: &mut Frame<'_>, confirm: &ConfirmState) {
    let area = frame.area();
    let block = Block::bordered().title("Confirm cleanup");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [body, buttons, help] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let (question, branch) = if confirm.worktree.branch.is_empty() {
        (
            "Remove this worktree?",
            "The branch is empty; only the worktree will be removed.".to_string(),
        )
    } else {
        (
            "Remove this worktree and branch?",
            format!("Branch: {}", clean_text(&confirm.worktree.branch)),
        )
    };
    let content = format!(
        "{question}\nPath: {}\n{}",
        clean_text(&confirm.worktree.path.display().to_string()),
        branch
    );
    frame.render_widget(Paragraph::new(content).wrap(Wrap { trim: true }), body);
    let cancel = if confirm.confirm {
        "Cancel"
    } else {
        "[ Cancel ]"
    };
    let accept = if confirm.confirm {
        "[ Remove ]"
    } else {
        "Remove"
    };
    frame.render_widget(Paragraph::new(format!("{cancel}    {accept}")), buttons);
    frame.render_widget(
        Paragraph::new("Tab/←/→ choose • Enter accept • Esc cancel")
            .style(Style::default().fg(Color::DarkGray)),
        help,
    );
}

fn draw_result(frame: &mut Frame<'_>, result: &ResultState) {
    let area = frame.area();
    let block = Block::bordered().title(if result.failed { "Failed" } else { "Result" });
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [title, body, help] = result_layout(inner);
    frame.render_widget(
        Paragraph::new(clean_text(&result.title)).style(if result.failed {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().add_modifier(Modifier::BOLD)
        }),
        title,
    );
    let paragraph = result_paragraph(result);
    let scroll = result.scroll.min(max_scroll(&paragraph, body));
    frame.render_widget(paragraph.scroll((scroll, 0)), body);
    frame.render_widget(
        Paragraph::new(HELP_RESULT).style(Style::default().fg(Color::DarkGray)),
        help,
    );
}

fn result_paragraph(result: &ResultState) -> Paragraph<'static> {
    Paragraph::new(clean_text(&result.body)).wrap(Wrap { trim: false })
}

fn centered_rect(area: Rect, width_percent: u16, height_percent: u16) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - height_percent) / 2),
        Constraint::Percentage(height_percent),
        Constraint::Percentage((100 - height_percent) / 2),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Percentage((100 - width_percent) / 2),
        Constraint::Percentage(width_percent),
        Constraint::Percentage((100 - width_percent) / 2),
    ])
    .split(vertical[1]);
    horizontal[1]
}

fn clean_text(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\n' => '\n',
            '\t' => ' ',
            '\r' => ' ',
            ch if ch.is_control() => '�',
            ch => ch,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::draw;
    use crate::aws::Profile;
    use crate::tui::app::{Action, App, Resource, Screen, ToolId};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn rendered(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .iter()
            .map(|cell| cell.symbol().to_string())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn wrapped_result_scroll_clamps_state_and_moves_back_immediately() {
        let mut app = App::new("/tmp/project".into());
        app.finished(
            &Action::CreateWorktree,
            "Created".into(),
            "界".repeat(150),
            false,
        );
        for _ in 0..100 {
            app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        }
        super::update_scroll_bounds(&mut app, ratatui::layout::Rect::new(0, 0, 20, 8));
        let Screen::Result(result) = &app.screen else {
            panic!("expected result")
        };
        let bottom = result.scroll;
        assert!(bottom > 0 && bottom < 100);
        app.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        let Screen::Result(result) = &app.screen else {
            panic!("expected result")
        };
        assert_eq!(result.scroll, bottom - 1);
        super::update_scroll_bounds(&mut app, ratatui::layout::Rect::new(0, 0, 100, 30));
        let Screen::Result(result) = &app.screen else {
            panic!("expected result")
        };
        assert_eq!(result.scroll, 0);
    }

    #[test]
    fn renders_home_and_result_with_test_backend() {
        let app = App::new("/tmp/project".into());
        let home = rendered(&app, 80, 20);
        assert!(home.contains("AWS: select profile"));
        assert!(home.contains("/tmp/project"));

        let mut result_app = App::new("/tmp/project".into());
        result_app.finished(
            &Action::CreateWorktree,
            "Created".into(),
            "line one\nline two".into(),
            false,
        );
        let result = rendered(&result_app, 80, 20);
        assert!(result.contains("Created"));
        assert!(result.contains("line one"));
    }

    #[test]
    fn renders_confirmation_and_tiny_terminal_without_panicking() {
        let mut app = App::new("/tmp/project".into());
        app.screen = Screen::ConfirmCleanup(crate::tui::app::ConfirmState {
            worktree: crate::git::AgentWorktree {
                name: "x".into(),
                path: PathBuf::from("/tmp/x"),
                branch: String::new(),
            },
            confirm: false,
            return_picker: crate::tui::app::PickerState::new(
                crate::tui::app::ToolId::GitWorktreeCleanup,
            ),
        });
        let text = rendered(&app, 2, 1);
        assert_eq!(text.chars().count(), 2);
    }

    #[test]
    fn renders_picker_errors_and_no_matches() {
        let mut error_app = App::new("/tmp/project".into());
        error_app.loaded(ToolId::AwsProfile, Err("profiles unavailable".into()));
        let error_text = rendered(&error_app, 60, 12);
        assert!(error_text.contains("profiles unavailable"));

        let mut empty_app = App::new("/tmp/project".into());
        empty_app.loaded(
            ToolId::AwsProfile,
            Ok(vec![Resource::Aws(Profile {
                name: "production".into(),
                settings: Default::default(),
            })]),
        );
        empty_app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE));
        let empty_text = rendered(&empty_app, 60, 12);
        assert!(empty_text.contains("No matches"));
    }

    #[test]
    fn confirmation_mentions_branch_removal() {
        let mut app = App::new("/tmp/project".into());
        app.screen = Screen::ConfirmCleanup(crate::tui::app::ConfirmState {
            worktree: crate::git::AgentWorktree {
                name: "x".into(),
                path: PathBuf::from("/tmp/x"),
                branch: "x-branch".into(),
            },
            confirm: false,
            return_picker: crate::tui::app::PickerState::new(ToolId::GitWorktreeCleanup),
        });
        assert!(rendered(&app, 80, 12).contains("Remove this worktree and branch?"));
    }

    #[test]
    fn replaces_terminal_controls_in_results() {
        let mut app = App::new("/tmp/project".into());
        app.finished(
            &Action::CreateWorktree,
            "Title\u{1b}".into(),
            "safe\u{7}".into(),
            false,
        );
        let text = rendered(&app, 40, 8);
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{7}'));
    }
}
