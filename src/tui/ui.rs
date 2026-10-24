use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Padding, Paragraph, Wrap};

use super::app::{
    App, Category, ConfirmState, DetailsState, FormField, LaunchForm, MenuState, NewWorktree,
    PickerState, ProjectCommand, ProjectField, ProjectForm, ProjectState, Resource, ResultState,
    Screen, TailorState, ToolId, branch_label,
};

/// Rows used by the launcher below the shell prompt.
pub(super) const HEIGHT: u16 = 12;

const ACCENT: Color = Color::Cyan;
const HELP_SEARCH: &str = "↑↓ move · ⏎ run · F1 help · esc back";
const HELP_PICKER: &str = "type to filter · ⏎ select · F5 refresh · F1 help · esc back";
const HELP_WORKTREES: &str = "type to filter · ⏎ open · ^A new · ^D remove · F1 help · esc back";
const HELP_PROJECTS: &str =
    "type to filter · ⏎ open · ^A new · ^S/^R/^X sync/restart/stop · F1 help · esc back";
const HELP_VMS: &str = "type to filter · ⏎/^X stop · ^D prune · F5 refresh · F1 help · esc back";
const HELP_MENU: &str = "key or ⏎ run · ↑↓ move · ? help · esc back";
const HELP_TAILOR: &str = "a agent · e edit · s skip · ? help";
const HELP_RETAILOR: &str = "a agent · e edit · ? help · esc back";
const HELP_NEW_WORKTREE: &str = "tab name/prompt · ←→ agent · ⏎ create · F1 help · esc back";
const HELP_PROJECT_FORM: &str = "tab ↑↓ field · ←→ space change · ⏎ create · F1 help · esc back";
const HELP_RESULT: &str = "↑↓ scroll · ⏎/esc back · ? help";
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
        Screen::Confirm(confirm) => {
            let [body, _] = confirm_layout(inner);
            confirm.scroll = confirm
                .scroll
                .min(max_scroll(&confirm_paragraph(confirm), body));
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
        Screen::ProjectMenu(menu) => draw_menu(frame, menu),
        Screen::TailorFlake(tailor) => draw_tailor(frame, tailor),
        Screen::NewWorktree(form) => draw_new_worktree(frame, app, form),
        Screen::ProjectForm(form) => draw_project_form(frame, form),
        Screen::Confirm(confirm) => draw_confirmation(frame, confirm),
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
    if state.status.worktrees.is_empty() {
        "a add · u start · v review saved · m more · ? help · esc back"
    } else if state.status.vm.is_some() {
        "v review · x stop · a add · ⏎ cd · D done · d remove · m more · ? help"
    } else {
        "v review saved · u start · a add · ⏎ cd · D done · d remove · m more · ? help"
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
            if picker.tool == ToolId::GitWorktrees {
                entries.extend([
                    entry("⏎", "worktree details"),
                    entry("^A", "new worktree (init + Herdr)"),
                    entry("^D", "remove worktree"),
                ]);
            } else if picker.tool == ToolId::SandboxProjects {
                entries.extend([
                    entry("⏎", "project details"),
                    entry("^A", "new project for the launch directory's repository"),
                    entry("^S", "sync the highlighted project"),
                    entry("^R", "restart the highlighted project's VM (asks first)"),
                    entry("^X", "stop the highlighted project's VM (asks first)"),
                ]);
            } else if picker.tool == ToolId::SandboxVms {
                entries.extend([
                    entry(
                        "⏎ ^X",
                        "stop the VM and remove its herdr machine (asks first)",
                    ),
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
            entry("x", "stop the VM (asks first)"),
            entry(
                "v",
                "review selected agent in lazygit (separate host checkout)",
            ),
            entry("a", "add a worktree (mounted on the next start)"),
            entry(
                "⏎ c",
                if app.print_shell {
                    "cd into the worktree and exit"
                } else {
                    "show the cd command"
                },
            ),
            entry(
                "D",
                "done: save its work as snapshots and remove it (branch kept unless accepted)",
            ),
            entry(
                "d",
                "remove the worktree without saving (asks; keep or delete the branch)",
            ),
            entry("m", "more: sync, restart, tailor, update, prune"),
            entry("esc", "back to the list"),
        ],
        Screen::ProjectMenu(_) => {
            let mut entries: Vec<_> = ProjectCommand::ALL
                .iter()
                .map(|command| (command.key().to_string(), command.description()))
                .collect();
            entries.extend([
                entry("↑↓ j k", "move"),
                entry("⏎", "run the highlighted command"),
                entry("esc", "back to the project"),
            ]);
            entries
        }
        Screen::TailorFlake(_) => vec![
            entry(
                "a",
                "a coding agent tailors flake.nix (agent-sandbox setup instructions)",
            ),
            entry("e", "edit flake.nix in $VISUAL or $EDITOR"),
            entry("s esc", "skip and open the project (new projects)"),
            entry("esc", "back to the project (existing projects)"),
        ],
        Screen::NewWorktree(_) => vec![
            entry("type", "the name (the branch), or the agent's first prompt"),
            entry("tab ↑↓", "switch between the name and the prompt"),
            entry("←→", "choose the agent, or none"),
            entry("⌫ ^W ^U", "delete a character / word / all"),
            entry("⏎", "create the worktree (needs a name)"),
            entry("esc", "back without creating"),
        ],
        Screen::ProjectForm(_) => vec![
            entry("type", "the project name"),
            entry("tab ↑↓", "next / previous field"),
            entry(
                "←→ space",
                "change the template, or turn proxy / backlog on or off",
            ),
            entry("⏎", "create the project (no worktree yet)"),
            entry("esc", "back without creating"),
        ],
        Screen::Confirm(confirm) => {
            let mut entries: Vec<_> = confirm
                .choices
                .iter()
                .map(|choice| (choice.key.to_string(), "the choice of that name"))
                .collect();
            entries.extend([
                entry("n esc", "cancel"),
                entry("←→ tab", "choose a button"),
                entry("⏎", "accept the choice"),
                entry("↑↓ PgUp PgDn", "scroll"),
            ]);
            entries
        }
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
        ToolId::GitWorktrees => HELP_WORKTREES,
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
    } else if picker.resources.is_empty() && picker.tool == ToolId::GitWorktrees {
        Some("No git worktrees yet. Press Ctrl-A to create one.".to_string())
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
        breadcrumb(ToolId::GitWorktrees.category(), "Worktree"),
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

/// The project commands that have no key of their own on the details.
fn draw_menu(frame: &mut Frame<'_>, menu: &MenuState) {
    let area = frame.area();
    let project = &menu.project.status.project;
    let mut title = breadcrumb(Category::Sandbox, &project.name);
    title.extend([" › ".dark_gray(), Span::styled("More", Style::new().bold())]);
    let block = panel(title, project.dir.display().to_string(), HELP_MENU);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let width = ProjectCommand::ALL
        .iter()
        .map(|command| command.label().len())
        .max()
        .unwrap_or_default();
    let items: Vec<ListItem> = ProjectCommand::ALL
        .iter()
        .map(|command| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", command.key()),
                    Style::new().fg(ACCENT).bold(),
                ),
                Span::raw(format!(" {:<width$}  ", command.label())),
                Span::styled(command.description(), Style::new().fg(Color::DarkGray)),
            ]))
        })
        .collect();
    render_list(frame, inner, items, menu.selected);
}

fn draw_tailor(frame: &mut Frame<'_>, state: &TailorState) {
    let area = frame.area();
    let existing = state.back.is_some();
    let (context, help, question) = if existing {
        (
            "project",
            HELP_RETAILOR,
            "Tailor flake.nix again for changes in the repository?",
        )
    } else {
        (
            "new project",
            HELP_TAILOR,
            "Tailor flake.nix to the repository before the first start?",
        )
    };
    let block = panel(
        breadcrumb(Category::Sandbox, &state.project.name),
        context.into(),
        help,
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
        Line::from(question.bold()),
        Line::default(),
        option(
            "a",
            format!(
                "with {}, following agent-sandbox's setup instructions",
                state.agent
            ),
        ),
        option("e", format!("edit it in {}", state.editor)),
    ]);
    if !existing {
        lines.push(option("s", "skip; edit it later".into()));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_new_worktree(frame: &mut Frame<'_>, app: &App, form: &LaunchForm) {
    let area = frame.area();
    let (category, context) = match &form.target {
        NewWorktree::Local => (Category::Git, app.cwd_display.clone()),
        NewWorktree::Project(name) => (Category::Sandbox, format!("project {name}")),
    };
    let block = panel(
        breadcrumb(category, "New worktree"),
        context,
        HELP_NEW_WORKTREE,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let label = |name: &str| Span::styled(format!("{name:<8}"), Style::new().fg(ACCENT));
    let cursor = |field: FormField| {
        let shown = if form.focus == field { "▏" } else { "" };
        Span::styled(shown, Style::new().fg(ACCENT))
    };

    let [name, agent, prompt, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(form.agents)),
        Constraint::Min(u16::from(form.agents)),
        Constraint::Length(2),
    ])
    .spacing(1)
    .areas(inner);
    let mut spans = vec![label("Name")];
    if form.name.is_empty() {
        spans.push(cursor(FormField::Name));
        spans.push(" the branch, such as feature/login".dark_gray());
    } else {
        spans.push(Span::raw(clean_text(&form.name)));
        spans.push(cursor(FormField::Name));
        // The folder, mount and Herdr label when they differ from the branch.
        if let Ok(dir) = crate::git::worktree_dir_name(form.name.trim())
            && dir != form.name
        {
            spans.push(format!("  folder {dir}").dark_gray());
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), name);

    let hint_text = match form.note.as_deref() {
        Some(note) => Span::styled(clean_text(note), Style::new().fg(Color::Yellow)),
        None => clean_text(&new_worktree_hint(form)).dark_gray(),
    };
    frame.render_widget(Paragraph::new(hint_text).wrap(Wrap { trim: true }), hint);
    if !form.agents {
        return;
    }

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
    let paragraph = Paragraph::new(Line::from(vec![
        label("Prompt"),
        text,
        cursor(FormField::Prompt),
    ]))
    .wrap(Wrap { trim: false });
    // Keep the end of a long prompt, where typing happens, in view.
    let scroll = max_scroll(&paragraph, prompt);
    frame.render_widget(paragraph.scroll((scroll, 0)), prompt);
}

/// What Enter does in the new worktree form.
fn new_worktree_hint(form: &LaunchForm) -> String {
    if form.name.trim().is_empty() {
        return "Type a name to create the worktree.".into();
    }
    if !form.agents {
        return match form.target {
            NewWorktree::Local => "⏎ creates the worktree.".into(),
            NewWorktree::Project(_) => {
                "⏎ adds the worktree; the VM mounts it on its next start or restart.".into()
            }
        };
    }
    let with_prompt = if form.prompt.trim().is_empty() {
        ""
    } else {
        " with the prompt"
    };
    match (&form.target, form.kind()) {
        (NewWorktree::Local, Some(kind)) => {
            format!("⏎ creates the worktree, opens it in herdr and starts {kind}{with_prompt}.")
        }
        (NewWorktree::Local, None) => {
            "⏎ creates the worktree and opens it in herdr, without an agent.".into()
        }
        (NewWorktree::Project(_), Some(kind)) => format!(
            "⏎ adds the worktree. {kind} starts in it{with_prompt} when the VM opens it: u starts, r restarts a running VM."
        ),
        (NewWorktree::Project(_), None) if form.kinds.is_empty() => {
            "⏎ adds the worktree. The project's flake.nix enables no agents (ai.<kind>.enable).".into()
        }
        (NewWorktree::Project(_), None) => {
            "⏎ adds the worktree. The VM opens it in herdr with $CLI_TOOLBOX_AGENT, if the project enables it.".into()
        }
    }
}

fn draw_project_form(frame: &mut Frame<'_>, form: &ProjectForm) {
    let area = frame.area();
    let block = panel(
        breadcrumb(Category::Sandbox, "New project"),
        form.options.repo.display().to_string(),
        HELP_PROJECT_FORM,
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let row = |field: ProjectField, name: &str, value: Span<'static>| {
        let focused = form.focus == field;
        let marker = if focused { "▌" } else { " " };
        Line::from(vec![
            Span::styled(marker, Style::new().fg(ACCENT)),
            Span::styled(format!("{name:<10}"), Style::new().fg(ACCENT)),
            value,
        ])
    };
    let choice = |text: String| Span::styled(format!("‹ {text} ›"), Style::new().bold());
    let switch = |on: bool| choice(if on { "on" } else { "off" }.into());
    let name = if form.name.is_empty() {
        "the guest host name is agent-<name>".dark_gray()
    } else {
        Span::raw(clean_text(&form.name))
    };
    let mut lines = vec![
        row(ProjectField::Name, "Name", name),
        row(
            ProjectField::Template,
            "Template",
            choice(form.template().into()),
        ),
        row(ProjectField::Proxy, "Proxy", switch(form.proxy)),
        row(ProjectField::Backlog, "Backlog", switch(form.backlog)),
        Line::default(),
    ];
    let hint = if form.name.trim().is_empty() {
        "Type a name to create the project.".to_string()
    } else {
        format!(
            "⏎ creates projects/{}/flake.nix in the sandbox directory from the {} template. Proxy: agent credentials from agent-proxy. Backlog: a Backlog.md folder for the agents. Add worktrees afterwards.",
            form.name.trim(),
            form.template()
        )
    };
    lines.push(Line::from(clean_text(&hint).dark_gray()));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

fn field(name: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{name:<8}"), Style::new().fg(ACCENT)),
        Span::raw(clean_text(value)),
    ])
}

/// The key hints of a confirmation: its choices' keys, then cancel.
fn confirm_help(confirm: &ConfirmState) -> String {
    let mut keys: Vec<String> = confirm
        .choices
        .iter()
        .map(|choice| format!("{} {}", choice.key, choice.label.to_lowercase()))
        .collect();
    keys.extend(["n cancel".into(), "←→ ⏎ choose".into(), "? help".into()]);
    keys.join(" · ")
}

fn confirm_layout(inner: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
        .spacing(1)
        .areas(inner)
}

/// The question and its lines. `Label: value` lines with a one-word label
/// line up like fields.
fn confirm_paragraph(confirm: &ConfirmState) -> Paragraph<'static> {
    let mut content = vec![
        Line::from(clean_text(&confirm.title).bold()),
        Line::default(),
    ];
    content.extend(
        confirm
            .lines
            .iter()
            .map(|line| match line.split_once(": ") {
                Some((label, value)) if label.len() < 8 && !label.contains(' ') => {
                    field(label, value)
                }
                _ => Line::from(clean_text(line)),
            }),
    );
    Paragraph::new(content).wrap(Wrap { trim: false })
}

fn draw_confirmation(frame: &mut Frame<'_>, confirm: &ConfirmState) {
    let area = frame.area();
    let help = confirm_help(confirm);
    let block = panel(
        vec![
            Span::styled("cli-toolbox", Style::new().fg(ACCENT).bold()),
            " › ".dark_gray(),
            Span::styled("Confirm", Style::new().bold()),
        ],
        String::new(),
        &help,
    )
    .border_style(Style::new().fg(Color::Red));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [body, buttons] = confirm_layout(inner);
    let paragraph = confirm_paragraph(confirm);
    let scroll = confirm.scroll.min(max_scroll(&paragraph, body));
    frame.render_widget(paragraph.scroll((scroll, 0)), body);

    let button = |label: String, active: bool, color: Color| {
        if active {
            Span::styled(
                format!(" {label} "),
                Style::new().fg(Color::Black).bg(color).bold(),
            )
        } else {
            Span::styled(format!(" {label} "), Style::new().fg(Color::DarkGray))
        }
    };
    let mut spans = vec![button("Cancel".into(), confirm.selected == 0, Color::White)];
    for (index, choice) in confirm.choices.iter().enumerate() {
        spans.push(Span::raw("  "));
        spans.push(button(
            clean_text(&choice.label),
            confirm.selected == index + 1,
            Color::Red,
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), buttons);
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
            &Action::CreateWorktree("x".into(), None),
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
        assert!(home.contains(" g Git"));
        assert!(home.contains("/tmp/project"));

        let mut result_app = App::new("/tmp/project".into());
        result_app.finished(
            &Action::CreateWorktree("x".into(), None),
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
        app.confirm(crate::tui::app::ConfirmState::new(
            "Remove?",
            vec!["Path: /tmp/x".into()],
            Vec::new(),
        ));
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
    fn confirmations_show_their_lines_and_choices() {
        let worktree = crate::git::Worktree {
            name: "x".into(),
            path: PathBuf::from("/tmp/x"),
            branch: "x-branch".into(),
        };
        let mut app = App::new("/tmp/project".into());
        app.herdr = true;
        app.loaded(ToolId::GitWorktrees, Ok(vec![Resource::Worktree(worktree)]));
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let text = rendered(&app, 80, 12);
        for want in [
            "Remove this worktree and branch?",
            "Path    /tmp/x",
            "Branch  x-branch",
            "workspace for it is closed too",
            " Cancel    Remove ",
            "y remove · n cancel",
        ] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
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
            ToolId::GitWorktrees,
            Ok(vec![Resource::Worktree(crate::git::Worktree {
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
        app.loaded(ToolId::GitWorktrees, Ok(Vec::new()));
        let text = rendered(&app, 80, super::HEIGHT);
        assert!(text.contains("Press Ctrl-A to create one"), "{text}");
    }

    #[test]
    fn renders_projects_and_their_worktrees() {
        let worktree = crate::git::Worktree {
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
                worktrees_root: PathBuf::from("/wt"),
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
        assert!(list.contains("^S/^R/^X sync/restart/stop "), "{list}");

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let details = rendered(&app, 100, super::HEIGHT);
        for want in [
            "Repo    /p/demo",
            "VM      stopped · press u to start",
            "demo-agent-wt",
            "u start",
            "v review saved",
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
    fn renders_the_tailor_prompt_of_an_existing_project_without_skip() {
        let mut app = App::new("/tmp/project".into());
        app.loaded(
            ToolId::SandboxProjects,
            Ok(vec![Resource::Project(Box::new(
                crate::agent::vm::ProjectStatus {
                    project: crate::sandbox::project::Project {
                        name: "demo".into(),
                        dir: PathBuf::from("/sandbox/projects/demo"),
                        repo: Some(PathBuf::from("/p/demo")),
                        state_dir: PathBuf::from("/sandbox/state/demo"),
                        worktrees_root: PathBuf::from("/wt"),
                    },
                    worktrees: Vec::new(),
                    vm: None,
                    restart_needed: false,
                    error: None,
                },
            ))]),
        );
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let details = rendered(&app, 100, super::HEIGHT);
        assert!(details.contains("a add · u start"), "{details}");
        assert!(details.contains("m more"), "{details}");
        app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        let menu = rendered(&app, 100, super::HEIGHT);
        for want in [
            "demo › More",
            " s  sync     publish agent work",
            " t  tailor   tailor flake.nix again",
            " f  update   update the flake's inputs",
            " p  prune    prune review snapshots",
        ] {
            assert!(menu.contains(want), "missing {want}: {menu}");
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        let text = rendered(&app, 100, super::HEIGHT);
        for want in [
            "Project demo: /sandbox/projects/demo/flake.nix",
            "Tailor flake.nix again",
            "a  with claude",
            "esc back",
        ] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
        assert!(!text.contains("s  skip"), "{text}");
    }

    #[test]
    fn renders_the_new_worktree_form() {
        let mut app = App::new("~/repo".into());
        app.herdr = true;
        app.agent_kinds = vec!["claude".into(), "codex".into()];
        app.default_agent = Some("codex".into());
        app.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        app.loaded(ToolId::GitWorktrees, Ok(Vec::new()));
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        let text = rendered(&app, 100, super::HEIGHT);
        for want in [
            "New worktree",
            "Name    ▏ the branch, such as feature/login",
            "Agent   ‹ codex ›",
            "optional: what the agent should do first",
            "Type a name",
        ] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
        for ch in "feature/tests".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let text = rendered(&app, 100, super::HEIGHT);
        assert!(
            text.contains("Name    feature/tests▏  folder feature-tests"),
            "{text}"
        );
        assert!(text.contains("starts codex."), "{text}");
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        for ch in "Add tests".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        let text = rendered(&app, 100, super::HEIGHT);
        assert!(text.contains("Prompt  Add tests"), "{text}");
        assert!(text.contains("starts codex with the prompt."), "{text}");
    }

    #[test]
    fn renders_the_project_form() {
        let mut app = App::new("/tmp/project".into());
        app.project_form(Ok(crate::sandbox::project::CreateOptions {
            name: Some("demo".into()),
            repo: PathBuf::from("/p/demo"),
            template: Some("rust".into()),
            source: None,
            ssh_key: crate::sandbox::project::SshKey::Auto,
            proxy: true,
            backlog: false,
        }));
        let text = rendered(&app, 100, super::HEIGHT);
        for want in [
            "New project",
            "/p/demo",
            "▌Name      demo",
            "Template  ‹ rust ›",
            "Proxy     ‹ on ›",
            "Backlog   ‹ off ›",
            "projects/demo/flake.nix",
        ] {
            assert!(text.contains(want), "missing {want}: {text}");
        }
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
            &Action::CreateWorktree("x".into(), None),
            "Title\u{1b}".into(),
            "safe\u{7}".into(),
            false,
        );
        let text = rendered(&app, 40, 8);
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{7}'));
    }
}
