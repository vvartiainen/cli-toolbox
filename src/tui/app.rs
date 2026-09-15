use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::aws::Profile;
use crate::git::AgentWorktree;
use crate::kitty::Choice;
use crate::select;
use crate::ssh::Host;

/// One of the actions offered by the launcher.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ToolId {
    AwsProfile,
    KittySession,
    SshConnect,
    GitWorktreeCreate,
    GitWorktreeList,
    GitWorktreeCleanup,
}

impl ToolId {
    pub(crate) const ALL: [Self; 6] = [
        Self::AwsProfile,
        Self::KittySession,
        Self::SshConnect,
        Self::GitWorktreeCreate,
        Self::GitWorktreeList,
        Self::GitWorktreeCleanup,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::AwsProfile => "AWS: select profile",
            Self::KittySession => "Kitty: switch session",
            Self::SshConnect => "SSH: connect",
            Self::GitWorktreeCreate => "Git: create agent worktree",
            Self::GitWorktreeList => "Git: list agent worktrees",
            Self::GitWorktreeCleanup => "Git: clean up agent worktree",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::AwsProfile => "Check SSO login when needed and prepare an AWS_PROFILE export.",
            Self::KittySession => "Switch Kitty to a session from a file in your home directory.",
            Self::SshConnect => "Connect to a configured host using Kitty's SSH kitten.",
            Self::GitWorktreeCreate => "Create the next sibling worktree and branch.",
            Self::GitWorktreeList => "Inspect paths and branches for the launch directory.",
            Self::GitWorktreeCleanup => "Remove a selected worktree and its branch.",
        }
    }

    pub(crate) fn picker_title(self) -> &'static str {
        match self {
            Self::AwsProfile => "AWS profiles",
            Self::KittySession => "Kitty sessions",
            Self::SshConnect => "SSH hosts",
            Self::GitWorktreeCreate => "Git worktrees",
            Self::GitWorktreeList => "Git worktrees",
            Self::GitWorktreeCleanup => "Git worktrees",
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
}

impl Resource {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Aws(profile) => profile.name.clone(),
            Self::Kitty(choice) => choice.label.clone(),
            Self::Ssh(host) => host.name.clone(),
            Self::Worktree(worktree) => worktree_label(worktree),
        }
    }

    fn identity(&self) -> ResourceIdentity {
        match self {
            Self::Aws(profile) => ResourceIdentity::String(profile.name.clone()),
            Self::Kitty(choice) => ResourceIdentity::Path(choice.path.clone()),
            Self::Ssh(host) => ResourceIdentity::String(host.name.clone()),
            Self::Worktree(worktree) => ResourceIdentity::Path(worktree.path.clone()),
        }
    }

    fn matches_tool(&self, tool: ToolId) -> bool {
        matches!(
            (tool, self),
            (ToolId::AwsProfile, Self::Aws(_))
                | (ToolId::KittySession, Self::Kitty(_))
                | (ToolId::SshConnect, Self::Ssh(_))
                | (
                    ToolId::GitWorktreeList | ToolId::GitWorktreeCleanup,
                    Self::Worktree(_)
                )
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
    Cleanup(AgentWorktree),
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
    ConfirmCleanup(ConfirmState),
    Result(ResultState),
}

/// Pure launcher state. Discovery and process execution are performed by the runtime.
#[derive(Clone, Debug)]
pub(crate) struct App {
    pub(crate) cwd_display: String,
    pub(crate) screen: Screen,
    pub(crate) home_query: String,
    pub(crate) home_selected: usize,
    pub(crate) loading: Option<ToolId>,
}

impl App {
    pub(crate) fn new(cwd_display: String) -> Self {
        Self {
            cwd_display,
            screen: Screen::Home,
            home_query: String::new(),
            home_selected: 0,
            loading: None,
        }
    }

    pub(crate) fn home_visible_indices(&self) -> Vec<usize> {
        let labels: Vec<String> = ToolId::ALL
            .iter()
            .map(|tool| tool.label().to_string())
            .collect();
        select::filter_items(&self.home_query, &labels)
    }

    pub(crate) fn selected_tool(&self) -> Option<ToolId> {
        self.home_visible_indices()
            .get(self.home_selected)
            .copied()
            .and_then(|index| ToolId::ALL.get(index).copied())
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

        match self.screen {
            Screen::Home => self.handle_home_key(key),
            Screen::Picker(_) => self.handle_picker_key(key),
            Screen::WorktreeDetails(_) => self.handle_details_key(key),
            Screen::ConfirmCleanup(_) => self.handle_confirm_key(key),
            Screen::Result(_) => self.handle_result_key(key),
        }
    }

    fn handle_home_key(&mut self, key: KeyEvent) -> Effect {
        if let Some(effect) = self.handle_text_key(key, true) {
            return effect;
        }
        match key.code {
            KeyCode::Esc => Effect::Quit,
            KeyCode::Enter => {
                let Some(tool) = self.selected_tool() else {
                    return Effect::None;
                };
                if tool == ToolId::GitWorktreeCreate {
                    return Effect::Execute(Action::CreateWorktree);
                }
                self.screen = Screen::Picker(PickerState::new(tool));
                self.loading = Some(tool);
                Effect::Load(tool)
            }
            KeyCode::F(5) => Effect::None,
            _ => Effect::None,
        }
    }

    fn handle_picker_key(&mut self, key: KeyEvent) -> Effect {
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
                    (ToolId::GitWorktreeList, Resource::Worktree(worktree)) => {
                        let return_picker = self
                            .picker()
                            .cloned()
                            .unwrap_or_else(|| PickerState::new(ToolId::GitWorktreeList));
                        self.screen = Screen::WorktreeDetails(DetailsState {
                            worktree,
                            return_picker,
                            scroll: 0,
                        });
                        Effect::None
                    }
                    (ToolId::GitWorktreeCleanup, Resource::Worktree(worktree)) => {
                        let return_picker = self
                            .picker()
                            .cloned()
                            .unwrap_or_else(|| PickerState::new(ToolId::GitWorktreeCleanup));
                        self.screen = Screen::ConfirmCleanup(ConfirmState {
                            worktree,
                            confirm: false,
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

    fn handle_details_key(&mut self, key: KeyEvent) -> Effect {
        if let Screen::WorktreeDetails(details) = &mut self.screen {
            match key.code {
                KeyCode::Up => {
                    details.scroll = details.scroll.saturating_sub(1);
                    return Effect::None;
                }
                KeyCode::Down => {
                    details.scroll = details.scroll.saturating_add(1);
                    return Effect::None;
                }
                KeyCode::PageUp => {
                    details.scroll = details.scroll.saturating_sub(10);
                    return Effect::None;
                }
                KeyCode::PageDown => {
                    details.scroll = details.scroll.saturating_add(10);
                    return Effect::None;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                if let Screen::WorktreeDetails(details) = &self.screen {
                    self.screen = Screen::Picker(details.return_picker.clone());
                }
                Effect::None
            }
            _ => Effect::None,
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Effect {
        if let Screen::ConfirmCleanup(confirm) = &mut self.screen {
            match key.code {
                KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                    confirm.confirm = !confirm.confirm;
                    return Effect::None;
                }
                KeyCode::Esc => {
                    self.screen = Screen::Picker(confirm.return_picker.clone());
                    return Effect::None;
                }
                KeyCode::Enter => {
                    if confirm.confirm {
                        let worktree = confirm.worktree.clone();
                        return Effect::Execute(Action::Cleanup(worktree));
                    }
                    self.screen = Screen::Picker(confirm.return_picker.clone());
                    return Effect::None;
                }
                _ => {}
            }
        }
        Effect::None
    }

    fn handle_result_key(&mut self, key: KeyEvent) -> Effect {
        if let Screen::Result(result) = &mut self.screen {
            match key.code {
                KeyCode::Up => result.scroll = result.scroll.saturating_sub(1),
                KeyCode::Down => result.scroll = result.scroll.saturating_add(1),
                KeyCode::PageUp => result.scroll = result.scroll.saturating_sub(10),
                KeyCode::PageDown => result.scroll = result.scroll.saturating_add(10),
                KeyCode::Enter | KeyCode::Esc => {
                    let cleanup = matches!(result.action, Action::Cleanup(_));
                    if cleanup {
                        self.screen = Screen::Picker(PickerState::new(ToolId::GitWorktreeCleanup));
                        self.loading = Some(ToolId::GitWorktreeCleanup);
                        return Effect::Load(ToolId::GitWorktreeCleanup);
                    }
                    self.screen = Screen::Home;
                    self.loading = None;
                }
                _ => {}
            }
        }
        Effect::None
    }

    fn handle_text_key(&mut self, key: KeyEvent, home: bool) -> Option<Effect> {
        let (query, selected, visible_len) = if home {
            let visible = self.home_visible_indices();
            (&mut self.home_query, &mut self.home_selected, visible.len())
        } else {
            return None;
        };
        text_key(key, query, selected, visible_len)
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
        KeyCode::Up => {
            move_selection(selected, visible_len, -1);
            Some(Effect::None)
        }
        KeyCode::Down => {
            move_selection(selected, visible_len, 1);
            Some(Effect::None)
        }
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

fn move_selection(selected: &mut usize, visible_len: usize, delta: i32) {
    if visible_len == 0 {
        *selected = 0;
        return;
    }
    let next = *selected as i32 + delta;
    *selected = next.clamp(0, visible_len as i32 - 1) as usize;
}

fn worktree_label(worktree: &AgentWorktree) -> String {
    let branch = if worktree.branch.is_empty() {
        "(no branch)"
    } else {
        worktree.branch.as_str()
    };
    format!(
        "{}  {}  [{}]",
        worktree.name,
        worktree.path.display(),
        branch
    )
}

#[cfg(test)]
mod tests {
    use super::{Action, App, Effect, Resource, Screen, ToolId};
    use crate::aws::Profile;
    use crate::git::AgentWorktree;
    use crate::kitty::Choice;
    use crossterm::event::{KeyEvent, KeyEventKind, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: crossterm::event::KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn profile(name: &str) -> Resource {
        Resource::Aws(Profile {
            name: name.into(),
            settings: Default::default(),
        })
    }

    #[test]
    fn opening_and_selecting_a_filtered_resource_preserves_its_payload() {
        let mut app = App::new("/repo".into());
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Char('a'))),
            Effect::None
        );
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Enter)),
            Effect::Load(ToolId::AwsProfile)
        );
        app.loaded(
            ToolId::AwsProfile,
            Ok(vec![profile("prod"), profile("sandbox"), profile("admin")]),
        );
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Char('s'))),
            Effect::None
        );
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Enter)),
            Effect::Execute(Action::Aws(Profile {
                name: "sandbox".into(),
                settings: Default::default()
            }))
        );
    }

    #[test]
    fn refresh_keeps_selection_by_identity_when_labels_are_duplicate() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(crossterm::event::KeyCode::Down));
        app.handle_key(key(crossterm::event::KeyCode::Enter));
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
        app.handle_key(key(crossterm::event::KeyCode::Down));
        app.loaded(ToolId::KittySession, Ok(vec![second.clone(), first]));
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Enter)),
            Effect::Execute(Action::Kitty(PathBuf::from("two")))
        );
    }

    #[test]
    fn refresh_error_keeps_the_previous_resources_available() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(crossterm::event::KeyCode::Enter));
        let resource = profile("prod");
        app.loaded(ToolId::AwsProfile, Ok(vec![resource.clone()]));
        app.handle_key(key(crossterm::event::KeyCode::F(5)));
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
            app.handle_key(key(crossterm::event::KeyCode::Enter)),
            Effect::Execute(Action::Aws(Profile {
                name: "prod".into(),
                settings: Default::default()
            }))
        );
    }

    #[test]
    fn cleanup_defaults_to_cancel_and_requires_confirmation() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(crossterm::event::KeyCode::Down));
        for _ in 0..4 {
            app.handle_key(key(crossterm::event::KeyCode::Down));
        }
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Enter)),
            Effect::Load(ToolId::GitWorktreeCleanup)
        );
        let worktree = AgentWorktree {
            name: "repo-agent-wt".into(),
            path: PathBuf::from("/tmp/repo-agent-wt"),
            branch: "repo-agent-wt".into(),
        };
        app.loaded(
            ToolId::GitWorktreeCleanup,
            Ok(vec![Resource::Worktree(worktree.clone())]),
        );
        app.handle_key(key(crossterm::event::KeyCode::Enter));
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Enter)),
            Effect::None
        );
        // Cancel returned to the same picker. Open it again, then opt in.
        app.handle_key(key(crossterm::event::KeyCode::Enter));
        app.handle_key(key(crossterm::event::KeyCode::Right));
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Enter)),
            Effect::Execute(Action::Cleanup(worktree))
        );
    }

    #[test]
    fn result_return_refreshes_cleanup_and_other_results_return_home() {
        let mut app = App::new("/repo".into());
        let worktree = AgentWorktree {
            name: "repo-agent-wt".into(),
            path: PathBuf::from("/tmp/repo-agent-wt"),
            branch: String::new(),
        };
        let cleanup = Action::Cleanup(worktree);
        app.finished(&cleanup, "Cleaned".into(), "done".into(), false);
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Esc)),
            Effect::Load(ToolId::GitWorktreeCleanup)
        );
        app.finished(
            &Action::CreateWorktree,
            "Created".into(),
            "done".into(),
            false,
        );
        assert_eq!(
            app.handle_key(key(crossterm::event::KeyCode::Esc)),
            Effect::None
        );
        assert!(matches!(app.screen, Screen::Home));
    }

    #[test]
    fn key_releases_and_enter_repeats_do_not_execute() {
        let mut app = App::new("/repo".into());
        let mut release = key(crossterm::event::KeyCode::Enter);
        release.kind = KeyEventKind::Release;
        assert_eq!(app.handle_key(release), Effect::None);
        app.handle_key(key(crossterm::event::KeyCode::Enter));
        let mut repeat = key(crossterm::event::KeyCode::Enter);
        repeat.kind = KeyEventKind::Repeat;
        assert_eq!(app.handle_key(repeat), Effect::None);
    }

    #[test]
    fn unicode_filter_empty_results_and_back_preserve_home_state() {
        use crossterm::event::KeyCode;
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
        assert_eq!(app.home_query, "a");
        assert_eq!(app.selected_tool(), Some(ToolId::AwsProfile));
        assert_eq!(app.handle_key(key(KeyCode::Esc)), Effect::Quit);
    }

    #[test]
    fn all_catalog_entries_are_present() {
        assert_eq!(ToolId::ALL.len(), 6);
        assert!(ToolId::ALL.contains(&ToolId::GitWorktreeCleanup));
    }
}
