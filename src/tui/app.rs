use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::agent::vm::ProjectStatus;
use crate::aws::Profile;
use crate::git::AgentWorktree;
use crate::kitty::Choice;
use crate::select;
use crate::ssh::Host;

/// Rows moved by PageUp/PageDown in lists. Matches the list height of the
/// inline viewport.
const PAGE: isize = 8;

/// A group of related actions shown as one tab on the home screen.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Category {
    Aws,
    Agents,
    Sandbox,
    Terminal,
}

impl Category {
    pub(crate) const ALL: [Self; 4] = [Self::Aws, Self::Agents, Self::Sandbox, Self::Terminal];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Aws => "AWS",
            Self::Agents => "Agents",
            Self::Sandbox => "Sandbox",
            Self::Terminal => "Terminal",
        }
    }

    /// Key that switches the home screen to this category.
    pub(crate) fn hotkey(self) -> char {
        match self {
            Self::Aws => 'a',
            Self::Agents => 'i',
            Self::Sandbox => 's',
            Self::Terminal => 't',
        }
    }

    pub(crate) fn tools(self) -> Vec<ToolId> {
        ToolId::ALL
            .into_iter()
            .filter(|tool| tool.category() == self)
            .collect()
    }

    fn from_hotkey(ch: char) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.hotkey() == ch)
    }

    fn offset(self, delta: isize) -> Self {
        let len = Self::ALL.len() as isize;
        let index = Self::ALL
            .iter()
            .position(|category| *category == self)
            .unwrap_or_default() as isize;
        Self::ALL[(index + delta).rem_euclid(len) as usize]
    }
}

/// One of the actions offered by the launcher.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ToolId {
    AwsProfile,
    AgentWorktrees,
    SandboxProjects,
    KittySession,
    SshConnect,
}

impl ToolId {
    pub(crate) const ALL: [Self; 5] = [
        Self::AwsProfile,
        Self::AgentWorktrees,
        Self::SandboxProjects,
        Self::KittySession,
        Self::SshConnect,
    ];

    pub(crate) fn category(self) -> Category {
        match self {
            Self::AwsProfile => Category::Aws,
            Self::AgentWorktrees => Category::Agents,
            Self::SandboxProjects => Category::Sandbox,
            Self::KittySession | Self::SshConnect => Category::Terminal,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::AwsProfile => "Select profile",
            Self::KittySession => "Switch Kitty session",
            Self::SshConnect => "SSH connect",
            Self::AgentWorktrees => "Worktrees",
            Self::SandboxProjects => "Projects",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::AwsProfile => "SSO login when needed, then AWS_PROFILE export",
            Self::KittySession => "Switch to a .kitty-session file from your home directory",
            Self::SshConnect => "Connect to a configured host with Kitty's SSH kitten",
            Self::AgentWorktrees => "Create, inspect, cd into or remove agent worktrees",
            Self::SandboxProjects => {
                "Project VMs in ~/.agent-sandbox: create, start, add worktrees, sync"
            }
        }
    }

    pub(crate) fn picker_title(self) -> &'static str {
        match self {
            Self::AwsProfile => "Profiles",
            Self::KittySession => "Kitty sessions",
            Self::SshConnect => "SSH hosts",
            Self::AgentWorktrees => "Agent worktrees",
            Self::SandboxProjects => "Sandbox projects",
        }
    }
}

/// A value discovered for a resource picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Resource {
    Aws(Profile),
    Kitty(Choice),
    Ssh(Host),
    Worktree(AgentWorktree),
    Project(ProjectStatus),
}

impl Resource {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Aws(profile) => profile.name.clone(),
            Self::Kitty(choice) => choice.label.clone(),
            Self::Ssh(host) => host.name.clone(),
            Self::Worktree(worktree) => worktree_label(worktree),
            Self::Project(status) => status.project.name.clone(),
        }
    }

    fn identity(&self) -> ResourceIdentity {
        match self {
            Self::Aws(profile) => ResourceIdentity::String(profile.name.clone()),
            Self::Kitty(choice) => ResourceIdentity::Path(choice.path.clone()),
            Self::Ssh(host) => ResourceIdentity::String(host.name.clone()),
            Self::Worktree(worktree) => ResourceIdentity::Path(worktree.path.clone()),
            Self::Project(status) => ResourceIdentity::Path(status.project.dir.clone()),
        }
    }

    fn matches_tool(&self, tool: ToolId) -> bool {
        matches!(
            (tool, self),
            (ToolId::AwsProfile, Self::Aws(_))
                | (ToolId::KittySession, Self::Kitty(_))
                | (ToolId::SshConnect, Self::Ssh(_))
                | (ToolId::AgentWorktrees, Self::Worktree(_))
                | (ToolId::SandboxProjects, Self::Project(_))
        )
    }
}

/// A selected resource ready for the runtime to execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Aws(Profile),
    Kitty(PathBuf),
    Ssh(Host),
    CreateWorktree,
    EnterWorktree(AgentWorktree),
    Cleanup(AgentWorktree),
    /// A sandbox project for the launch directory's repository.
    CreateProject,
    /// The following name a sandbox project.
    StartProject(String),
    StopProject(String),
    RestartProject(String),
    SyncProject(String),
    AddProjectWorktree(String),
    /// Sync and remove a project's worktree, keeping its branch.
    DoneProjectWorktree(String, AgentWorktree),
    EnterProjectWorktree(AgentWorktree),
}

impl Action {
    /// The picker to reload after the result screen, if any. Other actions
    /// return to the home screen.
    pub(crate) fn return_tool(&self) -> Option<ToolId> {
        match self {
            Self::CreateWorktree | Self::EnterWorktree(_) | Self::Cleanup(_) => {
                Some(ToolId::AgentWorktrees)
            }
            Self::CreateProject
            | Self::StartProject(_)
            | Self::StopProject(_)
            | Self::RestartProject(_)
            | Self::SyncProject(_)
            | Self::AddProjectWorktree(_)
            | Self::DoneProjectWorktree(..)
            | Self::EnterProjectWorktree(_) => Some(ToolId::SandboxProjects),
            Self::Aws(_) | Self::Kitty(_) | Self::Ssh(_) => None,
        }
    }
}

/// Work requested by the state machine's owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Effect {
    None,
    Load(ToolId),
    Execute(Action),
    Quit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ResourceIdentity {
    String(String),
    Path(PathBuf),
}

#[derive(Clone, Debug)]
pub(crate) struct PickerState {
    pub(crate) tool: ToolId,
    pub(crate) resources: Vec<Resource>,
    pub(crate) query: String,
    pub(crate) selected: usize,
    pub(crate) error: Option<String>,
}

impl PickerState {
    pub(crate) fn new(tool: ToolId) -> Self {
        Self {
            tool,
            resources: Vec::new(),
            query: String::new(),
            selected: 0,
            error: None,
        }
    }

    pub(crate) fn visible_indices(&self) -> Vec<usize> {
        let labels: Vec<String> = self.resources.iter().map(Resource::label).collect();
        select::filter_items(&self.query, &labels)
            .into_iter()
            .filter(|index| {
                self.resources
                    .get(*index)
                    .is_some_and(|resource| resource.matches_tool(self.tool))
            })
            .collect()
    }

    fn selected_resource(&self) -> Option<&Resource> {
        self.visible_indices()
            .get(self.selected)
            .and_then(|index| self.resources.get(*index))
    }

    fn selected_worktree(&self) -> Option<AgentWorktree> {
        match self.selected_resource() {
            Some(Resource::Worktree(worktree)) => Some(worktree.clone()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ConfirmState {
    pub(crate) worktree: AgentWorktree,
    pub(crate) confirm: bool,
    pub(crate) return_picker: PickerState,
}

#[derive(Clone, Debug)]
pub(crate) struct DetailsState {
    pub(crate) worktree: AgentWorktree,
    pub(crate) return_picker: PickerState,
    pub(crate) scroll: u16,
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectState {
    pub(crate) status: ProjectStatus,
    /// The highlighted worktree.
    pub(crate) selected: usize,
    pub(crate) return_picker: PickerState,
}

impl ProjectState {
    pub(crate) fn selected_worktree(&self) -> Option<&AgentWorktree> {
        self.status.worktrees.get(self.selected)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResultState {
    pub(crate) action: Action,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) failed: bool,
    pub(crate) scroll: u16,
}

#[derive(Clone, Debug)]
pub(crate) enum Screen {
    Home,
    Picker(PickerState),
    WorktreeDetails(DetailsState),
    ProjectDetails(ProjectState),
    ConfirmCleanup(ConfirmState),
    Result(ResultState),
}

/// Pure launcher state. Discovery and process execution are performed by the runtime.
#[derive(Clone, Debug)]
pub(crate) struct App {
    pub(crate) cwd_display: String,
    pub(crate) screen: Screen,
    pub(crate) category: Category,
    /// Searching across all categories instead of browsing one.
    pub(crate) searching: bool,
    pub(crate) home_query: String,
    pub(crate) home_selected: usize,
    pub(crate) loading: Option<ToolId>,
    /// The key help overlay is drawn over the current screen.
    pub(crate) show_help: bool,
    /// Successful shell actions exit and print their command for `eval`.
    pub(crate) print_shell: bool,
    /// Herdr is installed, so worktree removal also closes its workspace.
    pub(crate) herdr: bool,
}

impl App {
    pub(crate) fn new(cwd_display: String) -> Self {
        Self {
            cwd_display,
            screen: Screen::Home,
            category: Category::Aws,
            searching: false,
            home_query: String::new(),
            home_selected: 0,
            loading: None,
            show_help: false,
            print_shell: false,
            herdr: false,
        }
    }

    /// Tools listed on the home screen: the active category, or every tool
    /// that matches the search.
    pub(crate) fn home_tools(&self) -> Vec<ToolId> {
        if !self.searching {
            return self.category.tools();
        }
        let labels: Vec<String> = ToolId::ALL
            .iter()
            .map(|tool| format!("{} {}", tool.category().label(), tool.label()))
            .collect();
        select::filter_items(&self.home_query, &labels)
            .into_iter()
            .filter_map(|index| ToolId::ALL.get(index).copied())
            .collect()
    }

    pub(crate) fn selected_tool(&self) -> Option<ToolId> {
        self.home_tools().get(self.home_selected).copied()
    }

    /// Feed a discovery result back into the state machine.
    pub(crate) fn loaded(&mut self, tool: ToolId, result: Result<Vec<Resource>, String>) {
        let old_picker = match &self.screen {
            Screen::Picker(picker) if picker.tool == tool => Some(picker.clone()),
            _ => None,
        };
        let mut picker = old_picker.unwrap_or_else(|| PickerState::new(tool));
        let old_identity = picker.selected_resource().map(Resource::identity);
        match result {
            Ok(resources) => {
                picker.resources = resources;
                picker.error = None;
            }
            Err(error) => {
                // Keep the previous list visible on a refresh failure. This
                // makes the error recoverable and preserves Back semantics.
                picker.error = Some(error);
            }
        }
        picker.selected = selected_position(&picker, old_identity.as_ref());
        self.screen = Screen::Picker(picker);
        self.loading = None;
    }

    /// Move to an action result. The result screen owns the action so that its
    /// return behavior remains deterministic even after the selected value is gone.
    pub(crate) fn finished(&mut self, action: &Action, title: String, body: String, failed: bool) {
        self.loading = None;
        self.screen = Screen::Result(ResultState {
            action: action.clone(),
            title,
            body,
            failed,
            scroll: 0,
        });
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Effect {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return Effect::None;
        }
        // An Enter repeat can otherwise execute an external command twice when
        // a terminal reports key auto-repeat events.
        if key.code == KeyCode::Enter && key.kind == KeyEventKind::Repeat {
            return Effect::None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            return Effect::Quit;
        }
        // Any key closes the help overlay without acting on the screen below.
        if self.show_help {
            self.show_help = false;
            return Effect::None;
        }
        if key.code == KeyCode::F(1)
            || (key.code == KeyCode::Char('?') && self.question_opens_help())
        {
            self.show_help = true;
            return Effect::None;
        }

        match self.screen {
            Screen::Home => self.handle_home_key(key),
            Screen::Picker(_) => self.handle_picker_key(key),
            Screen::WorktreeDetails(_) => self.handle_details_key(key),
            Screen::ProjectDetails(_) => self.handle_project_key(key),
            Screen::ConfirmCleanup(_) => self.handle_confirm_key(key),
            Screen::Result(_) => self.handle_result_key(key),
        }
    }

    /// `?` is text input while searching or filtering, so only F1 opens help there.
    fn question_opens_help(&self) -> bool {
        match self.screen {
            Screen::Home => !self.searching,
            Screen::Picker(_) => false,
            Screen::WorktreeDetails(_)
            | Screen::ProjectDetails(_)
            | Screen::ConfirmCleanup(_)
            | Screen::Result(_) => true,
        }
    }

    fn handle_home_key(&mut self, key: KeyEvent) -> Effect {
        if self.searching {
            return self.handle_search_key(key);
        }
        let visible_len = self.home_tools().len();
        if list_key(key, &mut self.home_selected, visible_len) {
            return Effect::None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (key.code, ctrl) {
            (KeyCode::Esc, _) | (KeyCode::Char('q'), false) => Effect::Quit,
            (KeyCode::Enter, _) => self.open_selected(),
            (KeyCode::Char('k'), false) | (KeyCode::Char('p'), true) => {
                move_selection(&mut self.home_selected, visible_len, -1);
                Effect::None
            }
            (KeyCode::Char('j'), false) | (KeyCode::Char('n'), true) => {
                move_selection(&mut self.home_selected, visible_len, 1);
                Effect::None
            }
            (KeyCode::Right | KeyCode::Tab, _) | (KeyCode::Char('l' | ']'), false) => {
                self.switch_category(self.category.offset(1));
                Effect::None
            }
            (KeyCode::Left | KeyCode::BackTab, _) | (KeyCode::Char('h' | '['), false) => {
                self.switch_category(self.category.offset(-1));
                Effect::None
            }
            (KeyCode::Char('/'), false) => {
                self.searching = true;
                self.home_query.clear();
                self.home_selected = 0;
                Effect::None
            }
            (KeyCode::Char(ch), false) => {
                if let Some(category) = Category::from_hotkey(ch) {
                    self.switch_category(category);
                    return Effect::None;
                }
                // Number keys run the n:th action of the active category.
                match ch.to_digit(10).and_then(|digit| digit.checked_sub(1)) {
                    Some(index) if (index as usize) < visible_len => {
                        self.home_selected = index as usize;
                        self.open_selected()
                    }
                    _ => Effect::None,
                }
            }
            _ => Effect::None,
        }
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> Effect {
        match key.code {
            KeyCode::Esc => {
                self.searching = false;
                self.home_query.clear();
                self.home_selected = 0;
                Effect::None
            }
            KeyCode::Enter => self.open_selected(),
            _ => {
                let visible_len = self.home_tools().len();
                text_key(
                    key,
                    &mut self.home_query,
                    &mut self.home_selected,
                    visible_len,
                )
                .unwrap_or(Effect::None)
            }
        }
    }

    fn switch_category(&mut self, category: Category) {
        if self.category != category {
            self.category = category;
            self.home_selected = 0;
        }
    }

    fn open_selected(&mut self) -> Effect {
        let Some(tool) = self.selected_tool() else {
            return Effect::None;
        };
        // Coming back from a search result lands in the tool's own category.
        self.category = tool.category();
        self.screen = Screen::Picker(PickerState::new(tool));
        self.loading = Some(tool);
        Effect::Load(tool)
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> Effect {
        if let Some(effect) = self.handle_worktree_picker_key(key) {
            return effect;
        }
        if let Some(effect) = self.handle_project_picker_key(key) {
            return effect;
        }
        if let Screen::Picker(picker) = &mut self.screen
            && let Some(effect) = picker_text_key(picker, key)
        {
            return effect;
        }

        match key.code {
            KeyCode::Esc => {
                self.loading = None;
                self.screen = Screen::Home;
                Effect::None
            }
            KeyCode::F(5) => {
                let Some(tool) = picker_tool(&self.screen) else {
                    return Effect::None;
                };
                self.loading = Some(tool);
                Effect::Load(tool)
            }
            KeyCode::Enter => {
                let Some((tool, resource)) = self.picker().and_then(|picker| {
                    picker
                        .selected_resource()
                        .map(|resource| (picker.tool, resource.clone()))
                }) else {
                    return Effect::None;
                };
                self.loading = None;
                match (tool, resource) {
                    (ToolId::AwsProfile, Resource::Aws(profile)) => {
                        Effect::Execute(Action::Aws(profile))
                    }
                    (ToolId::KittySession, Resource::Kitty(choice)) => {
                        Effect::Execute(Action::Kitty(choice.path))
                    }
                    (ToolId::SshConnect, Resource::Ssh(host)) => Effect::Execute(Action::Ssh(host)),
                    (ToolId::AgentWorktrees, Resource::Worktree(worktree)) => {
                        let return_picker = self
                            .picker()
                            .cloned()
                            .unwrap_or_else(|| PickerState::new(ToolId::AgentWorktrees));
                        self.screen = Screen::WorktreeDetails(DetailsState {
                            worktree,
                            return_picker,
                            scroll: 0,
                        });
                        Effect::None
                    }
                    (ToolId::SandboxProjects, Resource::Project(status)) => {
                        let return_picker = self
                            .picker()
                            .cloned()
                            .unwrap_or_else(|| PickerState::new(ToolId::SandboxProjects));
                        self.screen = Screen::ProjectDetails(ProjectState {
                            status,
                            selected: 0,
                            return_picker,
                        });
                        Effect::None
                    }
                    _ => Effect::None,
                }
            }
            _ => Effect::None,
        }
    }

    /// Ctrl shortcuts of the worktree manager. Plain keys filter the list.
    fn handle_worktree_picker_key(&mut self, key: KeyEvent) -> Option<Effect> {
        let picker = self
            .picker()
            .filter(|picker| picker.tool == ToolId::AgentWorktrees)?;
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match key.code {
            KeyCode::Char('a') => {
                self.loading = None;
                Some(Effect::Execute(Action::CreateWorktree))
            }
            KeyCode::Char('d') => {
                let Some(worktree) = picker.selected_worktree() else {
                    return Some(Effect::None);
                };
                let return_picker = picker.clone();
                self.loading = None;
                self.screen = Screen::ConfirmCleanup(ConfirmState {
                    worktree,
                    confirm: false,
                    return_picker,
                });
                Some(Effect::None)
            }
            _ => None,
        }
    }

    fn handle_details_key(&mut self, key: KeyEvent) -> Effect {
        let Screen::WorktreeDetails(details) = &mut self.screen else {
            return Effect::None;
        };
        match key.code {
            KeyCode::Up => details.scroll = details.scroll.saturating_sub(1),
            KeyCode::Down => details.scroll = details.scroll.saturating_add(1),
            KeyCode::PageUp => details.scroll = details.scroll.saturating_sub(10),
            KeyCode::PageDown => details.scroll = details.scroll.saturating_add(10),
            KeyCode::Enter | KeyCode::Char('c') => {
                return Effect::Execute(Action::EnterWorktree(details.worktree.clone()));
            }
            KeyCode::Char('d') => {
                self.screen = Screen::ConfirmCleanup(ConfirmState {
                    worktree: details.worktree.clone(),
                    confirm: false,
                    return_picker: details.return_picker.clone(),
                });
            }
            KeyCode::Esc => self.screen = Screen::Picker(details.return_picker.clone()),
            _ => {}
        }
        Effect::None
    }

    /// Ctrl shortcuts of the project list. Plain keys filter the list.
    fn handle_project_picker_key(&mut self, key: KeyEvent) -> Option<Effect> {
        self.picker()
            .filter(|picker| picker.tool == ToolId::SandboxProjects)?;
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match key.code {
            KeyCode::Char('a') => {
                self.loading = None;
                Some(Effect::Execute(Action::CreateProject))
            }
            _ => None,
        }
    }

    fn handle_project_key(&mut self, key: KeyEvent) -> Effect {
        let Screen::ProjectDetails(state) = &mut self.screen else {
            return Effect::None;
        };
        let count = state.status.worktrees.len();
        if list_key(key, &mut state.selected, count) {
            return Effect::None;
        }
        let name = state.status.project.name.clone();
        let worktree = state.selected_worktree().cloned();
        match key.code {
            KeyCode::Char('k') => move_selection(&mut state.selected, count, -1),
            KeyCode::Char('j') => move_selection(&mut state.selected, count, 1),
            KeyCode::Char('u') => return Effect::Execute(Action::StartProject(name)),
            KeyCode::Char('x') => return Effect::Execute(Action::StopProject(name)),
            KeyCode::Char('r') => return Effect::Execute(Action::RestartProject(name)),
            KeyCode::Char('s') => return Effect::Execute(Action::SyncProject(name)),
            KeyCode::Char('a') => return Effect::Execute(Action::AddProjectWorktree(name)),
            KeyCode::Enter | KeyCode::Char('c') => {
                if let Some(worktree) = worktree {
                    return Effect::Execute(Action::EnterProjectWorktree(worktree));
                }
            }
            KeyCode::Char('D') => {
                if let Some(worktree) = worktree {
                    return Effect::Execute(Action::DoneProjectWorktree(name, worktree));
                }
            }
            KeyCode::Esc => self.screen = Screen::Picker(state.return_picker.clone()),
            _ => {}
        }
        Effect::None
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Effect {
        let Screen::ConfirmCleanup(confirm) = &mut self.screen else {
            return Effect::None;
        };
        let remove = match key.code {
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                confirm.confirm = !confirm.confirm;
                return Effect::None;
            }
            KeyCode::Char('y') => true,
            KeyCode::Char('n') | KeyCode::Esc => false,
            KeyCode::Enter => confirm.confirm,
            _ => return Effect::None,
        };
        if remove {
            return Effect::Execute(Action::Cleanup(confirm.worktree.clone()));
        }
        self.screen = Screen::Picker(confirm.return_picker.clone());
        Effect::None
    }

    fn handle_result_key(&mut self, key: KeyEvent) -> Effect {
        let Screen::Result(result) = &mut self.screen else {
            return Effect::None;
        };
        match key.code {
            KeyCode::Up => result.scroll = result.scroll.saturating_sub(1),
            KeyCode::Down => result.scroll = result.scroll.saturating_add(1),
            KeyCode::PageUp => result.scroll = result.scroll.saturating_sub(10),
            KeyCode::PageDown => result.scroll = result.scroll.saturating_add(10),
            KeyCode::Enter | KeyCode::Esc => {
                if let Some(tool) = result.action.return_tool() {
                    self.screen = Screen::Picker(PickerState::new(tool));
                    self.loading = Some(tool);
                    return Effect::Load(tool);
                }
                self.screen = Screen::Home;
                self.loading = None;
            }
            _ => {}
        }
        Effect::None
    }

    fn picker(&self) -> Option<&PickerState> {
        match &self.screen {
            Screen::Picker(picker) => Some(picker),
            _ => None,
        }
    }
}

fn selected_position(picker: &PickerState, identity: Option<&ResourceIdentity>) -> usize {
    let visible = picker.visible_indices();
    if visible.is_empty() {
        return 0;
    }
    if let Some(identity) = identity
        && let Some(position) = visible.iter().position(|index| {
            picker
                .resources
                .get(*index)
                .is_some_and(|resource| resource.identity() == *identity)
        })
    {
        return position;
    }
    0
}

fn picker_tool(screen: &Screen) -> Option<ToolId> {
    match screen {
        Screen::Picker(picker) => Some(picker.tool),
        _ => None,
    }
}

fn picker_text_key(picker: &mut PickerState, key: KeyEvent) -> Option<Effect> {
    let visible_len = picker.visible_indices().len();
    text_key(key, &mut picker.query, &mut picker.selected, visible_len)
}

fn text_key(
    key: KeyEvent,
    query: &mut String,
    selected: &mut usize,
    visible_len: usize,
) -> Option<Effect> {
    if list_key(key, selected, visible_len) {
        return Some(Effect::None);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('n') => {
                move_selection(selected, visible_len, 1);
                return Some(Effect::None);
            }
            KeyCode::Char('p') => {
                move_selection(selected, visible_len, -1);
                return Some(Effect::None);
            }
            KeyCode::Char('u') => {
                query.clear();
                *selected = 0;
                return Some(Effect::None);
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Backspace => {
            query.pop();
            *selected = 0;
            Some(Effect::None)
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            query.push(ch);
            *selected = 0;
            Some(Effect::None)
        }
        _ => None,
    }
}

/// Arrow, page and Home/End navigation shared by every list. Returns whether
/// the key was consumed.
fn list_key(key: KeyEvent, selected: &mut usize, visible_len: usize) -> bool {
    match key.code {
        KeyCode::Up => move_selection(selected, visible_len, -1),
        KeyCode::Down => move_selection(selected, visible_len, 1),
        KeyCode::PageUp => move_selection(selected, visible_len, -PAGE),
        KeyCode::PageDown => move_selection(selected, visible_len, PAGE),
        KeyCode::Home => move_selection(selected, visible_len, isize::MIN),
        KeyCode::End => move_selection(selected, visible_len, isize::MAX),
        _ => return false,
    }
    true
}

fn move_selection(selected: &mut usize, visible_len: usize, delta: isize) {
    if visible_len == 0 {
        *selected = 0;
        return;
    }
    let next = (*selected as isize).saturating_add(delta);
    *selected = next.clamp(0, visible_len as isize - 1) as usize;
}

fn worktree_label(worktree: &AgentWorktree) -> String {
    format!(
        "{}  {}  [{}]",
        worktree.name,
        worktree.path.display(),
        branch_label(worktree)
    )
}

pub(crate) fn branch_label(worktree: &AgentWorktree) -> &str {
    if worktree.branch.is_empty() {
        "(no branch)"
    } else {
        worktree.branch.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, App, Category, Effect, Resource, Screen, ToolId};
    use crate::agent::vm::ProjectStatus;
    use crate::aws::Profile;
    use crate::git::AgentWorktree;
    use crate::kitty::Choice;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    fn profile(name: &str) -> Resource {
        Resource::Aws(Profile {
            name: name.into(),
            settings: Default::default(),
        })
    }

    fn worktree(name: &str) -> AgentWorktree {
        AgentWorktree {
            name: name.into(),
            path: PathBuf::from(format!("/tmp/{name}")),
            branch: name.into(),
        }
    }

    /// Open the worktree manager with the given worktrees loaded.
    fn worktree_manager(worktrees: &[AgentWorktree]) -> App {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('1'))),
            Effect::Load(ToolId::AgentWorktrees)
        );
        app.loaded(
            ToolId::AgentWorktrees,
            Ok(worktrees.iter().cloned().map(Resource::Worktree).collect()),
        );
        app
    }

    #[test]
    fn opening_and_selecting_a_filtered_resource_preserves_its_payload() {
        let mut app = App::new("/repo".into());
        assert_eq!(app.handle_key(key(KeyCode::Char('a'))), Effect::None);
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Load(ToolId::AwsProfile)
        );
        app.loaded(
            ToolId::AwsProfile,
            Ok(vec![profile("prod"), profile("sandbox"), profile("admin")]),
        );
        assert_eq!(app.handle_key(key(KeyCode::Char('s'))), Effect::None);
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::Aws(Profile {
                name: "sandbox".into(),
                settings: Default::default()
            }))
        );
    }

    #[test]
    fn refresh_keeps_selection_by_identity_when_labels_are_duplicate() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::Enter));
        let first = Resource::Kitty(Choice {
            label: "same".into(),
            path: PathBuf::from("one"),
        });
        let second = Resource::Kitty(Choice {
            label: "same".into(),
            path: PathBuf::from("two"),
        });
        app.loaded(
            ToolId::KittySession,
            Ok(vec![first.clone(), second.clone()]),
        );
        app.handle_key(key(KeyCode::Down));
        app.loaded(ToolId::KittySession, Ok(vec![second.clone(), first]));
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::Kitty(PathBuf::from("two")))
        );
    }

    #[test]
    fn refresh_error_keeps_the_previous_resources_available() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Enter));
        let resource = profile("prod");
        app.loaded(ToolId::AwsProfile, Ok(vec![resource.clone()]));
        app.handle_key(key(KeyCode::F(5)));
        app.loaded(
            ToolId::AwsProfile,
            Err("temporary discovery failure".into()),
        );
        assert!(matches!(
            &app.screen,
            Screen::Picker(picker)
                if picker.resources == vec![resource]
                    && picker.error.as_deref() == Some("temporary discovery failure")
        ));
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::Aws(Profile {
                name: "prod".into(),
                settings: Default::default()
            }))
        );
    }

    #[test]
    fn brackets_cycle_tabs_both_ways() {
        let mut app = App::new("/repo".into());
        for expected in [
            Category::Agents,
            Category::Sandbox,
            Category::Terminal,
            Category::Aws,
        ] {
            app.handle_key(key(KeyCode::Char(']')));
            assert_eq!(app.category, expected);
        }
        app.handle_key(key(KeyCode::Char('[')));
        assert_eq!(app.category, Category::Terminal);
    }

    #[test]
    fn agents_tab_opens_the_worktree_manager() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(app.category, Category::Agents);
        // Numbers beyond the category's actions do nothing.
        assert_eq!(app.handle_key(key(KeyCode::Char('2'))), Effect::None);
        assert_eq!(
            app.handle_key(key(KeyCode::Char('1'))),
            Effect::Load(ToolId::AgentWorktrees)
        );
    }

    #[test]
    fn worktree_details_cd_and_remove() {
        let tree = worktree("repo-agent-wt");
        let mut app = worktree_manager(std::slice::from_ref(&tree));
        app.handle_key(key(KeyCode::Enter));
        assert!(matches!(app.screen, Screen::WorktreeDetails(_)));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('c'))),
            Effect::Execute(Action::EnterWorktree(tree.clone()))
        );
        app.handle_key(key(KeyCode::Char('d')));
        assert!(matches!(app.screen, Screen::ConfirmCleanup(_)));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('y'))),
            Effect::Execute(Action::Cleanup(tree))
        );
    }

    fn project(name: &str, worktrees: &[AgentWorktree]) -> ProjectStatus {
        ProjectStatus {
            project: crate::sandbox::project::Project {
                name: name.into(),
                dir: PathBuf::from(format!("/sandbox/projects/{name}")),
                repo: Some(PathBuf::from(format!("/p/{name}"))),
                state_dir: PathBuf::from(format!("/sandbox/state/{name}")),
            },
            worktrees: worktrees.to_vec(),
            vm: None,
            restart_needed: false,
            error: None,
        }
    }

    #[test]
    fn sandbox_tab_lists_projects_and_drives_their_vm() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('s')));
        assert_eq!(app.category, Category::Sandbox);
        assert_eq!(
            app.handle_key(key(KeyCode::Char('1'))),
            Effect::Load(ToolId::SandboxProjects)
        );
        let first = worktree("demo-agent-wt");
        let second = worktree("demo-agent-wt-2");
        app.loaded(
            ToolId::SandboxProjects,
            Ok(vec![Resource::Project(project(
                "demo",
                &[first.clone(), second.clone()],
            ))]),
        );
        assert_eq!(
            app.handle_key(ctrl('a')),
            Effect::Execute(Action::CreateProject)
        );

        app.handle_key(key(KeyCode::Enter));
        assert!(matches!(app.screen, Screen::ProjectDetails(_)));
        for (ch, action) in [
            ('u', Action::StartProject("demo".into())),
            ('x', Action::StopProject("demo".into())),
            ('r', Action::RestartProject("demo".into())),
            ('s', Action::SyncProject("demo".into())),
            ('a', Action::AddProjectWorktree("demo".into())),
            ('c', Action::EnterProjectWorktree(first.clone())),
        ] {
            assert_eq!(
                app.handle_key(key(KeyCode::Char(ch))),
                Effect::Execute(action)
            );
        }
        app.handle_key(key(KeyCode::Down));
        assert_eq!(
            app.handle_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::SHIFT)),
            Effect::Execute(Action::DoneProjectWorktree("demo".into(), second))
        );
        app.handle_key(key(KeyCode::Esc));
        assert!(matches!(
            &app.screen,
            Screen::Picker(picker) if picker.tool == ToolId::SandboxProjects
        ));
    }

    #[test]
    fn manager_shortcuts_create_and_remove_with_cancel() {
        let tree = worktree("repo-agent-wt");
        let mut app = worktree_manager(std::slice::from_ref(&tree));
        assert_eq!(
            app.handle_key(ctrl('a')),
            Effect::Execute(Action::CreateWorktree)
        );
        app.handle_key(ctrl('d'));
        assert!(matches!(app.screen, Screen::ConfirmCleanup(_)));
        assert_eq!(app.handle_key(key(KeyCode::Char('n'))), Effect::None);
        assert!(matches!(
            &app.screen,
            Screen::Picker(picker) if picker.tool == ToolId::AgentWorktrees
        ));
    }

    #[test]
    fn cleanup_defaults_to_cancel_and_requires_confirmation() {
        let tree = worktree("repo-agent-wt");
        let mut app = worktree_manager(std::slice::from_ref(&tree));
        app.handle_key(ctrl('d'));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), Effect::None);
        // Cancel returned to the same picker. Open it again, then opt in.
        app.handle_key(ctrl('d'));
        app.handle_key(key(KeyCode::Right));
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::Cleanup(tree))
        );
    }

    #[test]
    fn worktree_results_reload_the_manager_and_other_results_return_home() {
        let mut app = App::new("/repo".into());
        for action in [
            Action::CreateWorktree,
            Action::EnterWorktree(worktree("a")),
            Action::Cleanup(worktree("a")),
        ] {
            app.finished(&action, "Done".into(), "done".into(), false);
            assert_eq!(
                app.handle_key(key(KeyCode::Esc)),
                Effect::Load(ToolId::AgentWorktrees)
            );
        }
        for action in [
            Action::CreateProject,
            Action::SyncProject("demo".into()),
            Action::DoneProjectWorktree("demo".into(), worktree("a")),
        ] {
            app.finished(&action, "Done".into(), "done".into(), false);
            assert_eq!(
                app.handle_key(key(KeyCode::Esc)),
                Effect::Load(ToolId::SandboxProjects)
            );
        }
        app.finished(
            &Action::Kitty(PathBuf::from("one")),
            "Switched".into(),
            "done".into(),
            false,
        );
        assert_eq!(app.handle_key(key(KeyCode::Esc)), Effect::None);
        assert!(matches!(app.screen, Screen::Home));
    }

    #[test]
    fn help_overlay_swallows_the_closing_key() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('?')));
        assert!(app.show_help);
        assert_eq!(app.handle_key(key(KeyCode::Char('q'))), Effect::None);
        assert!(!app.show_help);
        // In pickers `?` filters, and F1 opens help.
        app.handle_key(key(KeyCode::Enter));
        app.loaded(ToolId::AwsProfile, Ok(vec![profile("prod")]));
        app.handle_key(key(KeyCode::Char('?')));
        assert!(!app.show_help);
        app.handle_key(key(KeyCode::F(1)));
        assert!(app.show_help);
    }

    #[test]
    fn page_and_end_keys_clamp_to_the_list() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('t')));
        app.handle_key(key(KeyCode::End));
        assert_eq!(app.selected_tool(), Some(ToolId::SshConnect));
        app.handle_key(key(KeyCode::PageUp));
        assert_eq!(app.selected_tool(), Some(ToolId::KittySession));

        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Enter));
        let profiles = (0..20).map(|n| profile(&format!("p{n:02}"))).collect();
        app.loaded(ToolId::AwsProfile, Ok(profiles));
        app.handle_key(key(KeyCode::PageDown));
        app.handle_key(key(KeyCode::PageDown));
        app.handle_key(key(KeyCode::PageDown));
        let Screen::Picker(picker) = &app.screen else {
            panic!("expected picker")
        };
        assert_eq!(picker.selected, 19);
        app.handle_key(key(KeyCode::Home));
        let Screen::Picker(picker) = &app.screen else {
            panic!("expected picker")
        };
        assert_eq!(picker.selected, 0);
    }

    #[test]
    fn key_releases_and_enter_repeats_do_not_execute() {
        let mut app = App::new("/repo".into());
        let mut release = key(KeyCode::Enter);
        release.kind = KeyEventKind::Release;
        assert_eq!(app.handle_key(release), Effect::None);
        app.handle_key(key(KeyCode::Enter));
        let mut repeat = key(KeyCode::Enter);
        repeat.kind = KeyEventKind::Repeat;
        assert_eq!(app.handle_key(repeat), Effect::None);
    }

    #[test]
    fn unicode_filter_empty_results_and_back_preserve_home_state() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Enter));
        app.loaded(
            ToolId::AwsProfile,
            Ok(vec![profile("prod"), profile("東京")]),
        );
        app.handle_key(key(KeyCode::Char('東')));
        assert!(
            matches!(app.handle_key(key(KeyCode::Enter)), Effect::Execute(Action::Aws(profile)) if profile.name == "東京")
        );
        app.handle_key(key(KeyCode::Char('z')));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), Effect::None);
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.category, Category::Aws);
        assert_eq!(app.selected_tool(), Some(ToolId::AwsProfile));
        assert_eq!(app.handle_key(key(KeyCode::Esc)), Effect::Quit);
    }

    #[test]
    fn category_hotkey_and_number_run_an_action_directly() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('t')));
        assert_eq!(app.category, Category::Terminal);
        assert_eq!(
            app.handle_key(key(KeyCode::Char('2'))),
            Effect::Load(ToolId::SshConnect)
        );
    }

    #[test]
    fn search_spans_categories_and_esc_returns_to_tabs() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('/')));
        for ch in "ssh".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(app.selected_tool(), Some(ToolId::SshConnect));
        app.handle_key(key(KeyCode::Esc));
        assert!(!app.searching);
        assert_eq!(app.home_tools(), vec![ToolId::AwsProfile]);
        app.handle_key(key(KeyCode::Char('/')));
        for ch in "ssh".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Load(ToolId::SshConnect)
        );
        assert_eq!(app.category, Category::Terminal);
    }

    #[test]
    fn category_hotkeys_are_unique_and_not_navigation_keys() {
        let keys: Vec<char> = Category::ALL.iter().map(|c| c.hotkey()).collect();
        for (index, key) in keys.iter().enumerate() {
            assert!(!keys[index + 1..].contains(key));
            assert!(!"hjklq/[]?".contains(*key) && !key.is_ascii_digit());
        }
        for category in Category::ALL {
            assert!(!category.tools().is_empty() && category.tools().len() <= 9);
        }
    }
}
