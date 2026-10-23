use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Padding, Paragraph, Wrap};

use super::app::{
    App, Category, ConfirmState, DetailsState, LaunchForm, NewWorktree, PickerState, ProjectState,
    Resource, ResultState, Screen, TailorState, ToolId, branch_label,
};

/// Rows used by the launcher below the shell prompt.
pub(super) const HEIGHT: u16 = 12;

const ACCENT: Color = Color::Cyan;
const HELP_SEARCH: &str = "↑↓ move · ⏎ run · F1 help · esc back";
const HELP_PICKER: &str = "type to filter · ⏎ select · F5 refresh · F1 help · esc back";
const HELP_WORKTREES: &str = "type to filter · ⏎ open · ^A new · ^D remove · F1 help · esc back";
const HELP_PROJECTS: &str =
    "type to filter · ⏎ open · ^A new · ^S/^R/^X sync/restart/stop all · F1 help · esc back";
const HELP_VMS: &str = "type to filter · ⏎/^X stop · ^D prune · F5 refresh · F1 help · esc back";
const HELP_TAILOR: &str = "a agent · e edit · s skip · ? help";
const HELP_NEW_WORKTREE: &str = "←→ agent · type the prompt · ⏎ create · F1 help · esc back";
const HELP_RESULT: &str = "↑↓ scroll · ⏎/esc back · ? help";
const HELP_CONFIRM: &str = "y remove · n cancel · ←→ ⏎ choose · ? help";
const HELP_OVERLAY: &str = "any key closes help";

/// Update viewport-dependent state before drawing, including after a resize.
pub(super) fn update_scroll_bounds(app: &mut App, area: Rect) {
    let inner = panel(Vec::new(), String::new(), "").inner(area);
    match &mut app.screen {
        Screen::Result(result) => {
            let [_, body] = result_layout(inner);
            result.scroll = result
                .scroll
                .min(max_scroll(&result_paragraph(result), body));
        }
        Screen::WorktreeDetails(details) => {
            details.scroll = details
                .scroll
                .min(max_scroll(&details_paragraph(details), inner));
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

fn result_layout(inner: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Length(1), Constraint::Min(1)])
        .spacing(1)
        .areas(inner)
}

pub(crate) fn draw(frame: &mut Frame<'_>, app: &App) {
    if app.show_help {
        draw_help(frame, app);
        return;
    }
    match &app.screen {
        Screen::Home => draw_home(frame, app),
        Screen::Picker(picker) => draw_picker(frame, app, picker),
        Screen::WorktreeDetails(details) => draw_details(frame, app, details),
        Screen::ProjectDetails(project) => draw_project(frame, app, project),
        Screen::TailorFlake(tailor) => draw_tailor(frame, tailor),
        Screen::NewWorktree(form) => draw_new_worktree(frame, app, form),
        Screen::ConfirmCleanup(confirm) => draw_confirmation(frame, app, confirm),
        Screen::Result(result) => draw_result(frame, result),
    }
}

/// The shared frame: a rounded border with the location on the left, context
/// on the right and key hints in the bottom border.
fn panel<'a>(title: Vec<Span<'a>>, context: String, help: &'a str) -> Block<'a> {
    let mut spans = vec![Span::raw(" ")];
    spans.extend(title);
    spans.push(Span::raw(" "));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .padding(Padding::horizontal(1))
        .border_style(Style::new().fg(Color::DarkGray))
        .title_top(Line::from(spans))
        .title_bottom(Line::from(format!(" {help} ").dark_gray()));
    if context.is_empty() {
        return block;
    }
    block.title_top(Line::from(format!(" {} ", clean_text(&context)).dark_gray()).right_aligned())
}

fn breadcrumb(category: Category, page: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled("cli-toolbox", Style::new().fg(ACCENT).bold()),
        " › ".dark_gray(),
        Span::raw(category.label()),
        " › ".dark_gray(),
        Span::styled(page.to_string(), Style::new().bold()),
    ]
}

fn tab_keys() -> String {
    let keys: Vec<String> = Category::ALL
        .iter()
        .map(|category| category.hotkey().to_string())
        .collect();
    keys.join("/")
}

fn home_help() -> String {
    format!(
        "{} [ ] tabs · 1-9 run · / search · ? help · q quit",
        tab_keys()
    )
}

fn details_help(app: &App) -> &'static str {
    if app.print_shell {
        "⏎ cd & exit · d remove · ? help · esc back"
    } else {
        "⏎ cd here · d remove · ? help · esc back"
    }
}

fn project_help(state: &ProjectState) -> &'static str {
    if state.status.vm.is_some() {
        "x stop · r restart · s sync · a add · ⏎ cd · D done · ? help · esc back"
    } else {
        "u start · a add · ⏎ cd · D done · ? help · esc back"
    }
}

/// Every key of the current screen, shown by `?` or F1.
fn help_entries(app: &App) -> Vec<(String, &'static str)> {
    let entry = |keys: &str, what| (keys.to_string(), what);
    let mut entries = match &app.screen {
        Screen::Home if app.searching => vec![
            entry("type", "search all actions"),
            entry("↑↓ ^P ^N", "move"),
            entry("PgUp PgDn", "move by page"),
            entry("Home End", "first / last"),
            entry("⏎", "run the action"),
            entry("^U", "clear the search"),
            entry("esc", "back to tabs"),
        ],
        Screen::Home => vec![
            entry(&tab_keys(), "go to tab"),
            entry("[ ] ←→ h l", "previous / next tab"),
            entry("1-9", "run the n:th action"),
            entry("↑↓ j k", "move"),
            entry("Home End", "first / last"),
            entry("⏎", "run the action"),
            entry("/", "search all actions"),
            entry("q esc", "quit"),
        ],
        Screen::Picker(picker) => {
            let mut entries = vec![
                entry("type", "filter the list"),
                entry("↑↓ ^P ^N", "move"),
                entry("PgUp PgDn", "move by page"),
                entry("Home End", "first / last"),
                entry("^U", "clear the filter"),
                entry("F5", "refresh"),
                entry("esc", "back"),
            ];
            if picker.tool == ToolId::AgentWorktrees {
                entries.extend([
                    entry("⏎", "worktree details"),
                    entry("^A", "new worktree (init + Herdr)"),
                    entry("^D", "remove worktree"),
                ]);
            } else if picker.tool == ToolId::SandboxProjects {
                entries.extend([
                    entry("⏎", "project details"),
                    entry("^A", "new project for the launch directory's repository"),
                    entry("^S", "sync every project whose VM is running"),
                    entry("^R", "restart every running project VM"),
                    entry("^X", "stop every running project VM"),
                ]);
            } else if picker.tool == ToolId::SandboxVms {
                entries.extend([
                    entry("⏎ ^X", "stop the VM and remove its herdr machine"),
                    entry(
                        "^D",
                        "prune runtime directories of VMs whose directory is gone",
                    ),
                ]);
            } else {
                entries.push(entry("⏎", "select"));
            }
            entries
        }
        Screen::WorktreeDetails(_) => vec![
            entry(
                "⏎ c",
                if app.print_shell {
                    "cd into it and exit"
                } else {
                    "show the cd command"
                },
            ),
            entry("d", "remove worktree"),
            entry("↑↓ PgUp PgDn", "scroll"),
            entry("esc", "back to the list"),
        ],
        Screen::ProjectDetails(_) => vec![
            entry("↑↓ j k", "select a worktree"),
            entry("u", "start the VM and open the worktrees in herdr"),
            entry("x r", "stop / restart the VM"),
            entry("p", "set up agent-proxy credentials and the backlog"),
            entry("s", "sync the agents' commits to the branches"),
            entry("a", "add a worktree (mounted on the next start)"),
            entry(
                "⏎ c",
                if app.print_shell {
                    "cd into the worktree and exit"
                } else {
                    "show the cd command"
                },
            ),
            entry("D", "done: sync and remove the worktree; keep its branch"),
            entry("esc", "back to the list"),
        ],
        Screen::TailorFlake(_) => vec![
            entry(
                "a",
                "a coding agent tailors flake.nix (agent-sandbox setup instructions)",
            ),
            entry("e", "edit flake.nix in $VISUAL or $EDITOR"),
            entry("s esc", "skip and open the project"),
        ],
        Screen::NewWorktree(_) => vec![
            entry("←→ ↑↓ tab", "choose the agent, or none"),
            entry("type", "write its first prompt (optional)"),
            entry("⌫ ^W ^U", "delete a character / word / all"),
            entry("⏎", "create the worktree"),
            entry("esc", "back without creating"),
        ],
        Screen::ConfirmCleanup(_) => vec![
            entry("y", "remove"),
            entry("n esc", "cancel"),
            entry("←→ tab", "choose a button"),
            entry("⏎", "accept the choice"),
        ],
        Screen::Result(_) => vec![entry("↑↓ PgUp PgDn", "scroll"), entry("⏎ esc", "back")],
    };
    entries.push(entry("^C", "quit at any point"));
    entries
}

fn draw_help(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let block = panel(
        vec![
            Span::styled("cli-toolbox", Style::new().fg(ACCENT).bold()),
            " › ".dark_gray(),
            Span::styled("Keys", Style::new().bold()),
        ],
        String::new(),
        HELP_OVERLAY,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let entries = help_entries(app);
    let key_width = entries
        .iter()
        .map(|(keys, _)| keys.chars().count())
        .max()
        .unwrap_or_default();
    let line = |(keys, what): &(String, &str)| {
        Line::from(vec![
            Span::styled(format!("{keys:<key_width$}  "), Style::new().fg(ACCENT)),
            Span::raw(what.to_string()),
        ])
    };
    // Two columns keep every screen's keys inside the inline viewport.
    let rows = usize::from(inner.height.max(1));
    let columns: [Rect; 2] = if entries.len() > rows {
        Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)])
            .spacing(2)
            .areas(inner)
    } else {
        [inner, Rect::default()]
    };
    let (left, right) = entries.split_at(entries.len().min(rows));
    frame.render_widget(
        Paragraph::new(left.iter().map(line).collect::<Vec<_>>()),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(right.iter().map(line).collect::<Vec<_>>()),
        columns[1],
    );
}

fn draw_home(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    let help = if app.searching {
        HELP_SEARCH.to_string()
    } else {
        home_help()
    };
    let block = panel(
        vec![Span::styled("cli-toolbox", Style::new().fg(ACCENT).bold())],
        app.cwd_display.clone(),
        &help,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [header, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)])
        .spacing(1)
        .areas(inner);
    let header_line = if app.searching {
        search_line("/", &app.home_query)
    } else {
        tabs_line(app.category)
    };
    frame.render_widget(Paragraph::new(header_line), header);

    let tools = app.home_tools();
    if tools.is_empty() {
        frame.render_widget(
            Paragraph::new("No actions match this search.".dark_gray()),
            list,
        );
        return;
    }
    let label_width = tools.iter().map(|t| t.label().chars().count()).max();
    let label_width = label_width.unwrap_or_default() + 3;
    let items: Vec<ListItem> = tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let key = if app.searching {
                format!("{:<9}", tool.category().label())
            } else {
                format!("{}", index + 1)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {key}  "), Style::new().fg(ACCENT)),
                Span::raw(format!("{:<label_width$}", tool.label())),
                tool.description().dark_gray(),
            ]))
        })
        .collect();
    render_list(frame, list, items, app.home_selected);
}

fn tabs_line(active: Category) -> Line<'static> {
    let mut spans = Vec::new();
    for category in Category::ALL {
        let key = format!(" {} ", category.hotkey());
        let label = format!("{} ", category.label());
        if category == active {
            let style = Style::new().fg(Color::Black).bg(ACCENT);
            spans.push(Span::styled(key, style));
            spans.push(Span::styled(label, style.bold()));
        } else {
            spans.push(Span::styled(key, Style::new().fg(ACCENT)));
            spans.push(Span::raw(label));
        }
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

fn search_line(prompt: &str, query: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{prompt} "), Style::new().fg(ACCENT).bold()),
        Span::raw(clean_text(query)),
        Span::styled("▏", Style::new().fg(ACCENT)),
    ])
}

fn render_list(frame: &mut Frame<'_>, area: Rect, items: Vec<ListItem<'_>>, selected: usize) {
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(selected.min(items.len() - 1)));
    }
    let widget = List::new(items).highlight_symbol("▌").highlight_style(
        Style::new()
            .bg(Color::DarkGray)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    frame.render_stateful_widget(widget, area, &mut state);
}

fn draw_picker(frame: &mut Frame<'_>, app: &App, picker: &PickerState) {
    let area = frame.area();
    let visible = picker.visible_indices();
    let count = if app.loading.is_some() {
        "loading…".to_string()
    } else {
        format!("{}/{}", visible.len(), picker.resources.len())
    };
    let help = match picker.tool {
        ToolId::AgentWorktrees => HELP_WORKTREES,
        ToolId::SandboxProjects => HELP_PROJECTS,
        ToolId::SandboxVms => HELP_VMS,
        _ => HELP_PICKER,
    };
    let block = panel(
        breadcrumb(picker.tool.category(), picker.tool.picker_title()),
        count,
        help,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let message = if let Some(error) = &picker.error {
        Some(format!("Error: {}", clean_text(error)))
    } else if app.loading.is_some() {
        Some(format!(
            "Reading {}…",
            picker.tool.picker_title().to_ascii_lowercase()
        ))
    } else if picker.resources.is_empty() && picker.tool == ToolId::AgentWorktrees {
        Some("No agent worktrees yet. Press Ctrl-A to create one.".to_string())
    } else if picker.resources.is_empty() && picker.tool == ToolId::SandboxProjects {
        Some("No sandbox projects yet. Press Ctrl-A to create one for this repository.".to_string())
    } else if picker.resources.is_empty() && picker.tool == ToolId::SandboxVms {
        Some("No VMs running. Press Ctrl-D to prune stale runtime directories.".to_string())
    } else if picker.resources.is_empty() {
        Some(format!(
            "No {} found. Press F5 to refresh or Esc to go back.",
            picker.tool.picker_title().to_ascii_lowercase()
        ))
    } else if visible.is_empty() {
        Some("No matches. Press Backspace or Ctrl-U to change the search.".to_string())
    } else {
        None
    };
    let [search, list, message_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(u16::from(message.is_some())),
    ])
    .areas(inner);
    frame.render_widget(Paragraph::new(search_line("›", &picker.query)), search);

    let shown: Vec<&Resource> = visible
        .iter()
        .filter_map(|index| picker.resources.get(*index))
        .collect();
    let name_width = shown
        .iter()
        .filter_map(|resource| match resource {
            Resource::Worktree(worktree) => Some(clean_text(&worktree.name).chars().count()),
            Resource::Project(status) => Some(clean_text(&status.project.name).chars().count()),
            Resource::Vm(vm) => Some(clean_text(&vm.name).chars().count()),
            _ => None,
        })
        .max()
        .unwrap_or_default();
    let items: Vec<ListItem> = shown
        .into_iter()
        .map(|resource| ListItem::new(resource_line(resource, name_width)))
        .collect();
    render_list(frame, list, items, picker.selected);

    if let Some(message) = message {
        let color = if picker.error.is_some() {
            Color::Red
        } else {
            Color::Yellow
        };
        frame.render_widget(
            Paragraph::new(message).style(Style::new().fg(color)),
            message_area,
        );
    }
}

/// One picker row. Worktrees are aligned columns: name, branch, dimmed path.
fn resource_line(resource: &Resource, name_width: usize) -> Line<'static> {
    match resource {
        Resource::Worktree(worktree) => Line::from(vec![
            Span::raw(format!(" {:<name_width$}  ", clean_text(&worktree.name))),
            Span::styled(
                format!("[{}]  ", clean_text(branch_label(worktree))),
                Style::new().fg(ACCENT),
            ),
            clean_text(&worktree.path.display().to_string()).dark_gray(),
        ]),
        Resource::Project(status) => {
            let vm = match &status.vm {
                Some(vm) => vm
                    .ssh_port
                    .map(|port| format!("[running :{port}]  "))
                    .unwrap_or_else(|| "[running]  ".into()),
                None => "[stopped]  ".into(),
            };
            let repo = status
                .project
                .repo
                .as_ref()
                .map(|repo| repo.display().to_string())
                .unwrap_or_default();
            Line::from(vec![
                Span::raw(format!(
                    " {:<name_width$}  ",
                    clean_text(&status.project.name)
                )),
                Span::styled(vm, Style::new().fg(ACCENT)),
                Span::raw(format!("{} worktree(s)  ", status.worktrees.len())),
                clean_text(&repo).dark_gray(),
            ])
        }
        Resource::Vm(vm) => {
            let port = vm
                .ssh_port
                .map(|port| format!("[:{port}]  "))
                .unwrap_or_else(|| "[no ssh]  ".into());
            let mut spans = vec![
                Span::raw(format!(" {:<name_width$}  ", clean_text(&vm.name))),
                Span::styled(port, Style::new().fg(ACCENT)),
                Span::raw(format!("pid {}  ", vm.pid)),
                clean_text(&vm.dir.display().to_string()).dark_gray(),
            ];
            if !vm.dir.exists() {
                spans.push(Span::styled(
                    "  (directory gone)",
                    Style::new().fg(Color::Yellow),
                ));
            }
            Line::from(spans)
        }
        _ => Line::from(format!(" {}", clean_text(&resource.label()))),
    }
}

fn draw_details(frame: &mut Frame<'_>, app: &App, details: &DetailsState) {
    let area = frame.area();
    let block = panel(
        breadcrumb(ToolId::AgentWorktrees.category(), "Worktree"),
        String::new(),
        details_help(app),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let paragraph = details_paragraph(details);
    let scroll = details.scroll.min(max_scroll(&paragraph, inner));
    frame.render_widget(paragraph.scroll((scroll, 0)), inner);
}

fn details_paragraph(details: &DetailsState) -> Paragraph<'static> {
    let content = vec![
        field("Name", &details.worktree.name),
        field("Path", &details.worktree.path.display().to_string()),
        field("Branch", branch_label(&details.worktree)),
    ];
    Paragraph::new(Text::from(content)).wrap(Wrap { trim: true })
}

fn draw_project(frame: &mut Frame<'_>, app: &App, state: &ProjectState) {
    let area = frame.area();
    let status = &state.status;
    let block = panel(
        breadcrumb(Category::Sandbox, &status.project.name),
        status.project.dir.display().to_string(),
        project_help(state),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let repo = status
        .project
        .repo
        .as_ref()
        .map(|repo| repo.display().to_string())
        .unwrap_or_else(|| "(none: a hand-written flake)".into());
    let vm = match status.vm {
        Some(_) => status.vm_text(),
        None => "stopped · press u to start".into(),
    };
    let mut fields = vec![field("Repo", &repo), field("VM", &vm)];
    if let Some(error) = &status.error {
        fields.push(Line::from(clean_text(error)).style(Style::new().fg(Color::Red)));
    }
    let [top, list] = Layout::vertical([
        Constraint::Length(fields.len() as u16 + 1),
        Constraint::Min(1),
    ])
    .areas(inner);
    fields.push(Line::from("Worktrees".bold()));
    frame.render_widget(Paragraph::new(fields), top);
    if status.worktrees.is_empty() {
        let hint = if app.loading.is_some() {
            "Reading…"
        } else {
            "No worktrees. Press a to add one."
        };
        frame.render_widget(Paragraph::new(hint.yellow()), list);
        return;
    }
    let width = status
        .worktrees
        .iter()
        .map(|worktree| clean_text(&worktree.name).chars().count())
        .max()
        .unwrap_or_default();
    let items: Vec<ListItem> = status
        .worktrees
        .iter()
        .map(|worktree| ListItem::new(resource_line(&Resource::Worktree(worktree.clone()), width)))
        .collect();
    render_list(frame, list, items, state.selected);
}

fn draw_tailor(frame: &mut Frame<'_>, state: &TailorState) {
    let area = frame.area();
    let block = panel(
        breadcrumb(Category::Sandbox, &state.project.name),
        "new project".into(),
        HELP_TAILOR,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let option = |key: &str, text: String| {
        Line::from(vec![
            Span::styled(format!("  {key}  "), Style::new().fg(ACCENT).bold()),
            Span::raw(clean_text(&text)),
        ])
    };
    let mut lines: Vec<Line> = state
        .project
        .summary
        .lines()
        .map(|line| Line::from(clean_text(line)))
        .collect();
    lines.extend([
        Line::default(),
        Line::from("Tailor flake.nix to the repository before the first start?".bold()),
        Line::default(),
        option(
            "a",
            format!(
                "with {}, following agent-sandbox's setup instructions",
                state.agent
            ),
        ),
        option("e", format!("edit it in {}", state.editor)),
        option("s", "skip; edit it later".into()),
    ]);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_new_worktree(frame: &mut Frame<'_>, app: &App, form: &LaunchForm) {
    let area = frame.area();
    let (category, context) = match &form.target {
        NewWorktree::Local => (Category::Agents, app.cwd_display.clone()),
        NewWorktree::Project(name) => (Category::Sandbox, format!("project {name}")),
    };
    let block = panel(
        breadcrumb(category, "New worktree"),
        context,
        HELP_NEW_WORKTREE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [agent, prompt, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .spacing(1)
    .areas(inner);

    let label = |name: &str| Span::styled(format!("{name:<8}"), Style::new().fg(ACCENT));
    let kind = match form.kind() {
        Some(kind) => Span::styled(format!("‹ {} ›", clean_text(kind)), Style::new().bold()),
        None => "‹ none ›".dark_gray(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![label("Agent"), kind])),
        agent,
    );

    let text = if form.prompt.is_empty() {
        "optional: what the agent should do first".dark_gray()
    } else if form.kind.is_none() {
        Span::styled(clean_text(&form.prompt), Style::new().fg(Color::DarkGray))
    } else {
        Span::raw(clean_text(&form.prompt))
    };
    let cursor = Span::styled("▏", Style::new().fg(ACCENT));
    let paragraph =
        Paragraph::new(Line::from(vec![label("Prompt"), text, cursor])).wrap(Wrap { trim: false });
    // Keep the end of a long prompt, where typing happens, in view.
    let scroll = max_scroll(&paragraph, prompt);
    frame.render_widget(paragraph.scroll((scroll, 0)), prompt);

    let with_prompt = if form.prompt.trim().is_empty() {
        ""
    } else {
        " with the prompt"
    };
    let hint_text = match (&form.target, form.kind()) {
        (NewWorktree::Local, Some(kind)) => {
            format!("⏎ creates the worktree, opens it in herdr and starts {kind}{with_prompt}.")
        }
        (NewWorktree::Local, None) => {
            "⏎ creates the worktree and opens it in herdr, without an agent.".into()
        }
        (NewWorktree::Project(_), Some(kind)) => format!(
            "⏎ adds the worktree. {kind} starts in it{with_prompt} when the VM opens it: u starts, r restarts a running VM."
        ),
        (NewWorktree::Project(_), None) => {
            "⏎ adds the worktree. The VM opens it in herdr with $CLI_TOOLBOX_AGENT, if set.".into()
        }
    };
    frame.render_widget(
        Paragraph::new(clean_text(&hint_text).dark_gray()).wrap(Wrap { trim: true }),
        hint,
    );
}

fn field(name: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<8}"), Style::new().fg(ACCENT)),
        Span::raw(clean_text(value)),
    ])
}

fn draw_confirmation(frame: &mut Frame<'_>, app: &App, confirm: &ConfirmState) {
    let area = frame.area();
    let block = panel(
        breadcrumb(ToolId::AgentWorktrees.category(), "Confirm cleanup"),
        String::new(),
        HELP_CONFIRM,
    )
    .border_style(Style::new().fg(Color::Red));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [body, buttons] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
        .spacing(1)
        .areas(inner);
    let (question, branch) = if confirm.worktree.branch.is_empty() {
        (
            "Remove this worktree?",
            field("Branch", "(none; only the worktree will be removed)"),
        )
    } else {
        (
            "Remove this worktree and branch?",
            field("Branch", &confirm.worktree.branch),
        )
    };
    let mut content = vec![
        Line::from(question.bold()),
        Line::default(),
        field("Path", &confirm.worktree.path.display().to_string()),
        branch,
    ];
    if app.herdr {
        content.push(field("Herdr", "an open workspace for it is closed too"));
    }
    frame.render_widget(Paragraph::new(content).wrap(Wrap { trim: false }), body);

    let button = |label: &'static str, active: bool, color: Color| {
        if active {
            Span::styled(
                format!(" {label} "),
                Style::new().fg(Color::Black).bg(color).bold(),
            )
        } else {
            Span::styled(format!(" {label} "), Style::new().fg(Color::DarkGray))
        }
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            button("Cancel", !confirm.confirm, Color::White),
            Span::raw("  "),
            button("Remove", confirm.confirm, Color::Red),
        ])),
        buttons,
    );
}

fn draw_result(frame: &mut Frame<'_>, result: &ResultState) {
    let area = frame.area();
    let (status, color) = if result.failed {
        ("✗ Failed", Color::Red)
    } else {
        ("✓ Done", Color::Green)
    };
    let block = panel(
        vec![
            Span::styled("cli-toolbox", Style::new().fg(ACCENT).bold()),
            " › ".dark_gray(),
            Span::styled(status, Style::new().fg(color).bold()),
        ],
        String::new(),
        HELP_RESULT,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [title, body] = result_layout(inner);
    frame.render_widget(
        Paragraph::new(clean_text(&result.title))
            .style(Style::new().fg(color).add_modifier(Modifier::BOLD)),
        title,
    );
    let paragraph = result_paragraph(result);
    let scroll = result.scroll.min(max_scroll(&paragraph, body));
    frame.render_widget(paragraph.scroll((scroll, 0)), body);
}

fn result_paragraph(result: &ResultState) -> Paragraph<'static> {
    Paragraph::new(clean_text(&result.body)).wrap(Wrap { trim: false })
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
            &Action::CreateWorktree(None),
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
        let home = rendered(&app, 80, super::HEIGHT);
        assert!(home.contains("Select profile"));
        assert!(home.contains(" i Agents"));
        assert!(home.contains("/tmp/project"));

        let mut result_app = App::new("/tmp/project".into());
        result_app.finished(
            &Action::CreateWorktree(None),
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
            return_picker: crate::tui::app::PickerState::new(ToolId::AgentWorktrees),
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
            return_picker: crate::tui::app::PickerState::new(ToolId::AgentWorktrees),
        });
        assert!(rendered(&app, 80, 12).contains("Remove this worktree and branch?"));
        app.herdr = true;
        assert!(rendered(&app, 80, 12).contains("workspace for it is closed too"));
    }

    #[test]
    fn help_overlay_lists_tab_keys() {
        let mut app = App::new("/tmp/project".into());
        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        let text = rendered(&app, 80, super::HEIGHT);
        assert!(text.contains("[ ]"), "{text}");
        assert!(text.contains("go to tab"), "{text}");
    }

    #[test]
    fn worktrees_render_as_columns() {
        let mut app = App::new("/tmp/project".into());
        app.loaded(
            ToolId::AgentWorktrees,
            Ok(vec![Resource::Worktree(crate::git::AgentWorktree {
                name: "project-agent-wt".into(),
                path: PathBuf::from("/tmp/project-agent-wt"),
                branch: "feature".into(),
            })]),
        );
        let text = rendered(&app, 80, super::HEIGHT);
        assert!(
            text.contains("project-agent-wt  [feature]  /tmp/project-agent-wt"),
            "{text}"
        );
        assert!(text.contains("^D remove"), "{text}");
    }

    #[test]
    fn empty_worktree_list_points_to_create() {
        let mut app = App::new("/tmp/project".into());
        app.loaded(ToolId::AgentWorktrees, Ok(Vec::new()));
        let text = rendered(&app, 80, super::HEIGHT);
        assert!(text.contains("Press Ctrl-A to create one"), "{text}");
    }

    #[test]
    fn renders_projects_and_their_worktrees() {
        let worktree = crate::git::AgentWorktree {
            name: "demo-agent-wt".into(),
            path: PathBuf::from("/wt/demo-agent-wt"),
            branch: "demo-agent-wt".into(),
        };
        let status = crate::agent::vm::ProjectStatus {
            project: crate::sandbox::project::Project {
                name: "demo".into(),
                dir: PathBuf::from("/sandbox/projects/demo"),
                repo: Some(PathBuf::from("/p/demo")),
                state_dir: PathBuf::from("/sandbox/state/demo"),
            },
            worktrees: vec![worktree],
            vm: None,
            restart_needed: false,
            error: None,
        };
        let mut app = App::new("/tmp/project".into());
        app.loaded(
            ToolId::SandboxProjects,
            Ok(vec![Resource::Project(Box::new(status))]),
        );
        let list = rendered(&app, 100, super::HEIGHT);
        assert!(
            list.contains("demo  [stopped]  1 worktree(s)  /p/demo"),
            "{list}"
        );
        assert!(list.contains("^A new"), "{list}");
        assert!(list.contains("^S/^R/^X sync/restart/stop all"), "{list}");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let details = rendered(&app, 100, super::HEIGHT);
        for want in [
            "Repo    /p/demo",
            "VM      stopped · press u to start",
            "demo-agent-wt",
            "u start",
        ] {
            assert!(details.contains(want), "missing {want}: {details}");
        }

        let mut empty = App::new("/tmp/project".into());
        empty.loaded(ToolId::SandboxProjects, Ok(Vec::new()));
        assert!(rendered(&empty, 100, super::HEIGHT).contains("No sandbox projects yet"));
    }

    #[test]
    fn renders_the_tailor_prompt() {
        let mut app = App::new("/tmp/project".into());
        app.tailor_prompt(
            crate::tui::app::NewProject {
                name: "demo".into(),
                summary: "Sandbox project demo: /sandbox/projects/demo".into(),
            },
            "claude".into(),
            "nvim".into(),
        );
        let text = rendered(&app, 100, super::HEIGHT);
        for want in [
            "Sandbox project demo: /sandbox/projects/demo",
            "Tailor flake.nix to the repository",
            "a  with claude",
            "e  edit it in nvim",
            "s  skip",
        ] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
    }

    #[test]
    fn renders_the_new_worktree_form() {
        let mut app = App::new("~/repo".into());
        app.herdr = true;
        app.agent_kinds = vec!["claude".into(), "codex".into()];
        app.default_agent = Some("codex".into());
        app.handle_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        app.loaded(ToolId::AgentWorktrees, Ok(Vec::new()));
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let text = rendered(&app, 100, super::HEIGHT);
        for want in [
            "New worktree",
            "Agent   ‹ codex ›",
            "optional: what the agent should do first",
            "starts codex.",
        ] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
        for ch in "Add tests".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let text = rendered(&app, 100, super::HEIGHT);
        assert!(text.contains("Prompt  Add tests"), "{text}");
        assert!(text.contains("starts codex with the prompt."), "{text}");
    }

    #[test]
    fn renders_running_vms_and_flags_missing_directories() {
        let vm = crate::sandbox::RunnerState {
            pid: 4242,
            ssh_port: Some(2300),
            name: "agent-demo".into(),
            dir: PathBuf::from("/gone/projects/demo"),
            ..Default::default()
        };
        let mut app = App::new("/tmp/project".into());
        app.loaded(ToolId::SandboxVms, Ok(vec![Resource::Vm(Box::new(vm))]));
        let list = rendered(&app, 120, super::HEIGHT);
        assert!(
            list.contains("agent-demo  [:2300]  pid 4242  /gone/projects/demo  (directory gone)"),
            "{list}"
        );
        assert!(list.contains("^D prune"), "{list}");

        let mut empty = App::new("/tmp/project".into());
        empty.loaded(ToolId::SandboxVms, Ok(Vec::new()));
        assert!(rendered(&empty, 100, super::HEIGHT).contains("No VMs running"));
    }

    #[test]
    fn replaces_terminal_controls_in_results() {
        let mut app = App::new("/tmp/project".into());
        app.finished(
            &Action::CreateWorktree(None),
            "Title\u{1b}".into(),
            "safe\u{7}".into(),
            false,
        );
        let text = rendered(&app, 40, 8);
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{7}'));
    }
}
