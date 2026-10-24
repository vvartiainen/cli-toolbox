use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::agent::Launch;
use crate::agent::vm::ProjectStatus;
use crate::aws::Profile;
use crate::azure::{AccountAction, Subscription};
use crate::git::Worktree;
use crate::kitty::Choice as KittyChoice;
use crate::sandbox::RunnerState;
use crate::sandbox::project::{CreateOptions, TEMPLATES};
use crate::select;
use crate::ssh::Host;

/// Rows moved by PageUp/PageDown in lists. Matches the list height of the
/// inline viewport.
const PAGE: isize = 8;

/// Agent kinds to offer when Herdr does not list its own.
const FALLBACK_AGENTS: [&str; 3] = ["claude", "codex", "pi"];

/// A group of related actions shown as one tab on the home screen.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Category {
    Aws,
    Azure,
    Git,
    Sandbox,
    Terminal,
}

impl Category {
    pub(crate) const ALL: [Self; 5] = [
        Self::Aws,
        Self::Azure,
        Self::Git,
        Self::Sandbox,
        Self::Terminal,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Aws => "AWS",
            Self::Azure => "Azure",
            Self::Git => "Git",
            Self::Sandbox => "Sandbox",
            Self::Terminal => "Terminal",
        }
    }

    /// Key that switches the home screen to this category.
    pub(crate) fn hotkey(self) -> char {
        match self {
            Self::Aws => 'a',
            Self::Azure => 'z',
            Self::Git => 'g',
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
    AzureSubscription,
    AzureAccount,
    GitWorktrees,
    SandboxProjects,
    SandboxVms,
    KittySession,
    SshConnect,
}

impl ToolId {
    pub(crate) const ALL: [Self; 8] = [
        Self::AwsProfile,
        Self::AzureSubscription,
        Self::AzureAccount,
        Self::GitWorktrees,
        Self::SandboxProjects,
        Self::SandboxVms,
        Self::KittySession,
        Self::SshConnect,
    ];

    pub(crate) fn category(self) -> Category {
        match self {
            Self::AwsProfile => Category::Aws,
            Self::AzureSubscription | Self::AzureAccount => Category::Azure,
            Self::GitWorktrees => Category::Git,
            Self::SandboxProjects | Self::SandboxVms => Category::Sandbox,
            Self::KittySession | Self::SshConnect => Category::Terminal,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::AwsProfile => "Select profile",
            Self::AzureSubscription => "Select subscription",
            Self::AzureAccount => "Account",
            Self::KittySession => "Switch Kitty session",
            Self::SshConnect => "SSH connect",
            Self::GitWorktrees => "Worktrees",
            Self::SandboxProjects => "Projects",
            Self::SandboxVms => "VMs",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::AwsProfile => "SSO login when needed, then AWS_PROFILE export",
            Self::AzureSubscription => {
                "Login when needed, az account set, then AZURE/ARM_SUBSCRIPTION_ID export"
            }
            Self::AzureAccount => "Status, log in, log in again after PIM activation, log out",
            Self::KittySession => "Switch to a .kitty-session file from your home directory",
            Self::SshConnect => "Connect to a configured host with Kitty's SSH kitten",
            Self::GitWorktrees => {
                "Create, inspect, cd into or remove git worktrees in ~/.worktrees"
            }
            Self::SandboxProjects => {
                "Project VMs in ~/.agent-sandbox: create, start, add worktrees, sync"
            }
            Self::SandboxVms => "Every running VM, also those whose project is gone: stop, prune",
        }
    }

    pub(crate) fn picker_title(self) -> &'static str {
        match self {
            Self::AwsProfile => "Profiles",
            Self::AzureSubscription => "Subscriptions",
            Self::AzureAccount => "Azure account",
            Self::KittySession => "Kitty sessions",
            Self::SshConnect => "SSH hosts",
            Self::GitWorktrees => "Git worktrees",
            Self::SandboxProjects => "Sandbox projects",
            Self::SandboxVms => "Running VMs",
        }
    }
}

/// A value discovered for a resource picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Resource {
    Aws(Profile),
    AzureSubscription(Subscription),
    AzureAccount(AccountAction),
    Kitty(KittyChoice),
    Ssh(Host),
    Worktree(Worktree),
    Project(Box<ProjectStatus>),
    Vm(Box<RunnerState>),
}

impl Resource {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Aws(profile) => profile.name.clone(),
            Self::AzureSubscription(subscription) => subscription.label(),
            Self::AzureAccount(action) => action.label().into(),
            Self::Kitty(choice) => choice.label.clone(),
            Self::Ssh(host) => host.name.clone(),
            Self::Worktree(worktree) => worktree_label(worktree),
            Self::Project(status) => status.project.name.clone(),
            Self::Vm(vm) => vm.name.clone(),
        }
    }

    fn identity(&self) -> ResourceIdentity {
        match self {
            Self::Aws(profile) => ResourceIdentity::String(profile.name.clone()),
            Self::AzureSubscription(subscription) => {
                ResourceIdentity::String(subscription.id.clone())
            }
            Self::AzureAccount(action) => ResourceIdentity::String(action.label().into()),
            Self::Kitty(choice) => ResourceIdentity::Path(choice.path.clone()),
            Self::Ssh(host) => ResourceIdentity::String(host.name.clone()),
            Self::Worktree(worktree) => ResourceIdentity::Path(worktree.path.clone()),
            Self::Project(status) => ResourceIdentity::Path(status.project.dir.clone()),
            Self::Vm(vm) => ResourceIdentity::Path(vm.runtime_dir.clone()),
        }
    }

    fn matches_tool(&self, tool: ToolId) -> bool {
        matches!(
            (tool, self),
            (ToolId::AwsProfile, Self::Aws(_))
                | (ToolId::AzureSubscription, Self::AzureSubscription(_))
                | (ToolId::AzureAccount, Self::AzureAccount(_))
                | (ToolId::KittySession, Self::Kitty(_))
                | (ToolId::SshConnect, Self::Ssh(_))
                | (ToolId::GitWorktrees, Self::Worktree(_))
                | (ToolId::SandboxProjects, Self::Project(_))
                | (ToolId::SandboxVms, Self::Vm(_))
        )
    }
}

/// A selected resource ready for the runtime to execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Aws(Profile),
    AzureSubscription(Subscription),
    AzureAccount(AccountAction),
    Kitty(PathBuf),
    Ssh(Host),
    /// A git worktree of the launch directory by name, with the agent to
    /// start in it.
    CreateWorktree(String, Option<Launch>),
    EnterWorktree(Worktree),
    Cleanup(Worktree),
    /// A sandbox project for the launch directory's repository, as the
    /// project form chose.
    CreateProject(CreateOptions),
    /// The following name a sandbox project.
    StartProject(String),
    StopProject(String),
    RestartProject(String),
    /// Set up the host side of the project's proxy credentials and backlog.
    SetupProject(String),
    SyncProject(String),
    /// Open the selected agent worktree (or a saved snapshot) in lazygit.
    ReviewProject(String, Option<Worktree>),
    /// A worktree of the project by name, with the agent to start in it with
    /// the VM.
    AddProjectWorktree(String, String, Option<Launch>),
    /// Save a project worktree's work as snapshots, then remove it.
    DoneProjectWorktree(String, Worktree),
    /// Remove a project's worktree without saving its work; the flag keeps
    /// its branch.
    RemoveProjectWorktree(String, Worktree, bool),
    EnterProjectWorktree(Worktree),
    /// Tailor a new project's flake to its repository.
    TailorProject(String, Tailor),
    /// Stop the running VM with this runtime directory.
    StopVm(PathBuf),
    /// Remove the runtime directories of stopped VMs whose start directory is gone.
    PruneVms,
}

/// How to tailor a new project's flake.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Tailor {
    /// Run this coding agent with agent-sandbox's setup instructions.
    Agent(String),
    /// Open the flake in the editor.
    Editor,
}

/// A project that was just created, waiting for its flake to be tailored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NewProject {
    pub(crate) name: String,
    pub(crate) summary: String,
}

impl Action {
    /// The project whose details open after the result screen, if any.
    fn project_to_open(&self) -> Option<&str> {
        match self {
            Self::TailorProject(name, _)
            | Self::ReviewProject(name, _)
            | Self::RemoveProjectWorktree(name, ..) => Some(name),
            _ => None,
        }
    }

    /// The picker to reload after the result screen, if any. Other actions
    /// return to the home screen.
    pub(crate) fn return_tool(&self) -> Option<ToolId> {
        match self {
            Self::CreateWorktree(..) | Self::EnterWorktree(_) | Self::Cleanup(_) => {
                Some(ToolId::GitWorktrees)
            }
            Self::CreateProject(_)
            | Self::StartProject(_)
            | Self::StopProject(_)
            | Self::RestartProject(_)
            | Self::SetupProject(_)
            | Self::SyncProject(_)
            | Self::ReviewProject(..)
            | Self::AddProjectWorktree(..)
            | Self::DoneProjectWorktree(..)
            | Self::RemoveProjectWorktree(..)
            | Self::EnterProjectWorktree(_)
            | Self::TailorProject(..) => Some(ToolId::SandboxProjects),
            Self::StopVm(_) | Self::PruneVms => Some(ToolId::SandboxVms),
            Self::Aws(_)
            | Self::AzureSubscription(_)
            | Self::AzureAccount(_)
            | Self::Kitty(_)
            | Self::Ssh(_) => None,
        }
    }
}

/// Values the runtime looks up for a form, then hands to the matching
/// [`App`] method.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Prepare {
    /// The suggested name and the enabled agents of a new worktree of this
    /// project, for [`App::project_worktree_form`].
    ProjectWorktree(String),
    /// The project form's defaults for the launch directory's repository, for
    /// [`App::project_form`].
    Project,
    /// What the VM with this runtime directory mounts, for
    /// [`App::confirm_vm_stop`].
    StopVm(PathBuf),
}

/// Work requested by the state machine's owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Effect {
    None,
    Load(ToolId),
    /// Look up what a form needs before it opens; see [`Prepare`].
    Prepare(Prepare),
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

    fn selected_worktree(&self) -> Option<Worktree> {
        match self.selected_resource() {
            Some(Resource::Worktree(worktree)) => Some(worktree.clone()),
            _ => None,
        }
    }
}

/// One way to answer a confirmation.
#[derive(Clone, Debug)]
pub(crate) struct Choice {
    /// Picks it right away.
    pub(crate) key: char,
    pub(crate) label: String,
    pub(crate) action: Action,
}

/// A question before an action that is hard to undo. Cancel comes first and
/// is selected, so Enter alone does nothing.
#[derive(Clone, Debug)]
pub(crate) struct ConfirmState {
    pub(crate) title: String,
    pub(crate) lines: Vec<String>,
    pub(crate) choices: Vec<Choice>,
    /// 0 is Cancel, then the choices.
    pub(crate) selected: usize,
    /// Lines scrolled past, for long reports.
    pub(crate) scroll: u16,
    /// Where Cancel returns to.
    pub(crate) back: Box<Screen>,
}

impl ConfirmState {
    pub(crate) fn new(title: impl Into<String>, lines: Vec<String>, choices: Vec<Choice>) -> Self {
        Self {
            title: title.into(),
            lines,
            choices,
            selected: 0,
            scroll: 0,
            back: Box::new(Screen::Home),
        }
    }
}

impl Choice {
    pub(crate) fn new(key: char, label: impl Into<String>, action: Action) -> Self {
        Self {
            key,
            label: label.into(),
            action,
        }
    }
}

/// One line about a VM for confirmations: its name, SSH port, project folder
/// and how many worktrees it mounts.
pub(crate) fn vm_line(vm: &RunnerState, worktrees: Option<usize>) -> String {
    let port = vm
        .ssh_port
        .map(|port| format!("SSH port {port}"))
        .unwrap_or_else(|| "no SSH port".into());
    let worktrees = worktrees
        .map(|count| format!(", {count} worktree(s)"))
        .unwrap_or_default();
    format!("{}: {port}, {}{worktrees}", vm.name, vm.dir.display())
}

#[derive(Clone, Debug)]
pub(crate) struct DetailsState {
    pub(crate) worktree: Worktree,
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
    pub(crate) fn selected_worktree(&self) -> Option<&Worktree> {
        self.status.worktrees.get(self.selected)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TailorState {
    pub(crate) project: NewProject,
    /// The coding agent `a` runs.
    pub(crate) agent: String,
    /// The editor `e` opens, for the hint.
    pub(crate) editor: String,
}

/// What a VM confirmation is about.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VmChange {
    Stop,
    Restart,
}

impl VmChange {
    fn action(self, project: String) -> Action {
        match self {
            Self::Stop => Action::StopProject(project),
            Self::Restart => Action::RestartProject(project),
        }
    }
}

/// Where the new worktree of the form goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NewWorktree {
    /// The launch directory's repository; the agent starts right away.
    Local,
    /// The named sandbox project; the agent starts with its VM.
    Project(String),
}

/// The field of the new worktree form that typing edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FormField {
    Name,
    Prompt,
}

/// The name of a new worktree, and the agent and first prompt for it.
#[derive(Clone, Debug)]
pub(crate) struct LaunchForm {
    pub(crate) target: NewWorktree,
    /// The branch name; the folder name is derived from it.
    pub(crate) name: String,
    /// An agent can be chosen: Herdr is there to run it in.
    pub(crate) agents: bool,
    /// The agent kinds to choose from.
    pub(crate) kinds: Vec<String>,
    /// The chosen kind, or none for a worktree without an agent.
    pub(crate) kind: Option<usize>,
    pub(crate) prompt: String,
    pub(crate) focus: FormField,
    /// What could not be looked up for the form, such as an unreadable flake.
    pub(crate) note: Option<String>,
    /// Where Esc returns to.
    pub(crate) back: Box<Screen>,
}

impl LaunchForm {
    pub(crate) fn kind(&self) -> Option<&str> {
        self.kind
            .filter(|_| self.agents)
            .and_then(|index| self.kinds.get(index))
            .map(String::as_str)
    }

    fn launch(&self) -> Option<Launch> {
        self.kind()
            .map(|kind| Launch::new(kind, Some(self.prompt.clone())))
    }

    /// The worktree to create, once it has a name.
    fn action(&self) -> Option<Action> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        Some(match &self.target {
            NewWorktree::Local => Action::CreateWorktree(name.into(), self.launch()),
            NewWorktree::Project(project) => {
                Action::AddProjectWorktree(project.clone(), name.into(), self.launch())
            }
        })
    }

    fn field(&mut self) -> &mut String {
        match self.focus {
            FormField::Name => &mut self.name,
            FormField::Prompt => &mut self.prompt,
        }
    }

    /// Move between the name and the prompt; without agents there is only
    /// the name.
    fn switch_focus(&mut self) {
        self.focus = match self.focus {
            FormField::Name if self.agents => FormField::Prompt,
            _ => FormField::Name,
        };
    }

    /// Step through no agent and the kinds.
    fn cycle(&mut self, delta: isize) {
        let choices = self.kinds.len() as isize + 1;
        let current = self.kind.map_or(0, |index| index as isize + 1);
        let next = (current + delta).rem_euclid(choices);
        self.kind = (next > 0).then(|| next as usize - 1);
    }
}

/// The field of the project form that keys change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProjectField {
    Name,
    Template,
    Proxy,
    Backlog,
}

impl ProjectField {
    const ALL: [Self; 4] = [Self::Name, Self::Template, Self::Proxy, Self::Backlog];

    fn offset(self, delta: isize) -> Self {
        let index = Self::ALL
            .iter()
            .position(|field| *field == self)
            .unwrap_or(0);
        Self::ALL[(index as isize + delta).rem_euclid(Self::ALL.len() as isize) as usize]
    }
}

/// The options of a new sandbox project.
#[derive(Clone, Debug)]
pub(crate) struct ProjectForm {
    /// The defaults, with the repository and SSH key that the form keeps.
    pub(crate) options: CreateOptions,
    pub(crate) name: String,
    /// An index into [`TEMPLATES`].
    pub(crate) template: usize,
    pub(crate) proxy: bool,
    pub(crate) backlog: bool,
    pub(crate) focus: ProjectField,
    /// Where Esc returns to.
    pub(crate) back: Box<Screen>,
}

impl ProjectForm {
    pub(crate) fn template(&self) -> &'static str {
        TEMPLATES[self.template % TEMPLATES.len()]
    }

    fn action(&self) -> Option<Action> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        Some(Action::CreateProject(CreateOptions {
            name: Some(name.into()),
            template: Some(self.template().into()),
            proxy: self.proxy,
            backlog: self.backlog,
            ..self.options.clone()
        }))
    }

    /// Change the focused choice: the template cycles, the switches flip.
    fn change(&mut self, delta: isize) {
        match self.focus {
            ProjectField::Name => {}
            ProjectField::Template => {
                let count = TEMPLATES.len() as isize;
                self.template = (self.template as isize + delta).rem_euclid(count) as usize;
            }
            ProjectField::Proxy => self.proxy = !self.proxy,
            ProjectField::Backlog => self.backlog = !self.backlog,
        }
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
    ProjectDetails(Box<ProjectState>),
    TailorFlake(TailorState),
    NewWorktree(LaunchForm),
    ProjectForm(ProjectForm),
    Confirm(ConfirmState),
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
    /// Herdr is installed, so worktree removal also closes its workspace and
    /// new worktrees can start an agent.
    pub(crate) herdr: bool,
    /// The agent kinds Herdr can start.
    pub(crate) agent_kinds: Vec<String>,
    /// The agent new worktrees start with unless changed: `CLI_TOOLBOX_AGENT`.
    pub(crate) default_agent: Option<String>,
    /// Open this project's details once the project list has loaded.
    pub(crate) open_project: Option<String>,
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
            agent_kinds: Vec::new(),
            default_agent: None,
            open_project: None,
        }
    }

    /// Ask for the name of a new worktree and, with Herdr to run one in, the
    /// agent to start there with its first prompt. `kinds` are the agents on
    /// offer; the default one is chosen when it is among them.
    fn new_worktree(
        &mut self,
        target: NewWorktree,
        name: String,
        kinds: Vec<String>,
        note: Option<String>,
        back: Screen,
    ) {
        self.loading = None;
        let kind = self
            .default_agent
            .as_ref()
            .and_then(|default| kinds.iter().position(|kind| kind == default));
        self.screen = Screen::NewWorktree(LaunchForm {
            target,
            name,
            agents: self.herdr,
            kinds,
            kind,
            prompt: String::new(),
            focus: FormField::Name,
            note,
            back: Box::new(back),
        });
    }

    /// The agents a git worktree can start: those Herdr lists (or common
    /// ones), and the default.
    fn local_agent_kinds(&self) -> Vec<String> {
        let mut kinds = self.agent_kinds.clone();
        if kinds.is_empty() {
            kinds = FALLBACK_AGENTS.map(String::from).to_vec();
        }
        if let Some(default) = &self.default_agent
            && !kinds.contains(default)
        {
            kinds.insert(0, default.clone());
        }
        kinds
    }

    /// Open the new worktree form of `project` with what the runtime looked
    /// up: the suggested `name` and the `agents` the project enables, the only
    /// ones it offers. `note` says what could not be looked up.
    pub(crate) fn project_worktree_form(
        &mut self,
        project: String,
        name: String,
        agents: Vec<String>,
        note: Option<String>,
    ) {
        let back = self.screen.clone();
        self.new_worktree(NewWorktree::Project(project), name, agents, note, back);
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
        self.loading = None;
        let open = self
            .open_project
            .take()
            .filter(|_| tool == ToolId::SandboxProjects);
        let status = open.and_then(|name| {
            picker.resources.iter().find_map(|resource| match resource {
                Resource::Project(status) if status.project.name == name => Some(status.clone()),
                _ => None,
            })
        });
        self.screen = match status {
            Some(status) => Screen::ProjectDetails(Box::new(ProjectState {
                status: *status,
                selected: 0,
                return_picker: picker,
            })),
            None => Screen::Picker(picker),
        };
    }

    /// Open the project form with `defaults` for the launch directory's
    /// repository, or show why there are none on the project list.
    pub(crate) fn project_form(&mut self, defaults: Result<CreateOptions, String>) {
        self.loading = None;
        let options = match defaults {
            Ok(options) => options,
            Err(error) => {
                if let Screen::Picker(picker) = &mut self.screen {
                    picker.error = Some(error);
                }
                return;
            }
        };
        let template = options
            .template
            .as_deref()
            .and_then(|template| TEMPLATES.iter().position(|known| *known == template))
            .unwrap_or(0);
        self.screen = Screen::ProjectForm(ProjectForm {
            name: options.name.clone().unwrap_or_default(),
            template,
            proxy: options.proxy,
            backlog: options.backlog,
            focus: ProjectField::Name,
            back: Box::new(self.screen.clone()),
            options,
        });
    }

    /// Ask how to tailor the flake of a project that was just created.
    pub(crate) fn tailor_prompt(&mut self, project: NewProject, agent: String, editor: String) {
        self.loading = None;
        self.screen = Screen::TailorFlake(TailorState {
            project,
            agent,
            editor,
        });
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
            Screen::TailorFlake(_) => self.handle_tailor_key(key),
            Screen::NewWorktree(_) => self.handle_new_worktree_key(key),
            Screen::ProjectForm(_) => self.handle_project_form_key(key),
            Screen::Confirm(_) => self.handle_confirm_key(key),
            Screen::Result(_) => self.handle_result_key(key),
        }
    }

    /// `?` is text input while searching, filtering or writing a prompt, so
    /// only F1 opens help there.
    fn question_opens_help(&self) -> bool {
        match self.screen {
            Screen::Home => !self.searching,
            Screen::Picker(_) | Screen::NewWorktree(_) | Screen::ProjectForm(_) => false,
            Screen::WorktreeDetails(_)
            | Screen::ProjectDetails(_)
            | Screen::TailorFlake(_)
            | Screen::Confirm(_)
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
        if let Some(effect) = self.handle_vm_picker_key(key) {
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
                    (ToolId::AzureSubscription, Resource::AzureSubscription(subscription)) => {
                        Effect::Execute(Action::AzureSubscription(subscription))
                    }
                    (ToolId::AzureAccount, Resource::AzureAccount(action)) => {
                        Effect::Execute(Action::AzureAccount(action))
                    }
                    (ToolId::KittySession, Resource::Kitty(choice)) => {
                        Effect::Execute(Action::Kitty(choice.path))
                    }
                    (ToolId::SshConnect, Resource::Ssh(host)) => Effect::Execute(Action::Ssh(host)),
                    (ToolId::GitWorktrees, Resource::Worktree(worktree)) => {
                        let return_picker = self
                            .picker()
                            .cloned()
                            .unwrap_or_else(|| PickerState::new(ToolId::GitWorktrees));
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
                        self.screen = Screen::ProjectDetails(Box::new(ProjectState {
                            status: *status,
                            selected: 0,
                            return_picker,
                        }));
                        Effect::None
                    }
                    (ToolId::SandboxVms, Resource::Vm(vm)) => {
                        Effect::Prepare(Prepare::StopVm(vm.runtime_dir))
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
            .filter(|picker| picker.tool == ToolId::GitWorktrees)?;
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        match key.code {
            KeyCode::Char('a') => {
                let back = Screen::Picker(picker.clone());
                let kinds = self.local_agent_kinds();
                self.new_worktree(NewWorktree::Local, String::new(), kinds, None, back);
                Some(Effect::None)
            }
            KeyCode::Char('d') => {
                let Some(worktree) = picker.selected_worktree() else {
                    return Some(Effect::None);
                };
                self.loading = None;
                self.confirm_cleanup(worktree);
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
                let worktree = details.worktree.clone();
                self.confirm_cleanup(worktree);
            }
            KeyCode::Esc => self.screen = Screen::Picker(details.return_picker.clone()),
            _ => {}
        }
        Effect::None
    }

    /// Ctrl shortcuts of the project list. Plain keys filter the list.
    fn handle_project_picker_key(&mut self, key: KeyEvent) -> Option<Effect> {
        let picker = self
            .picker()
            .filter(|picker| picker.tool == ToolId::SandboxProjects)?;
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        let status = match picker.selected_resource() {
            Some(Resource::Project(status)) => Some(status.as_ref().clone()),
            _ => None,
        };
        let effect = match (key.code, status) {
            (KeyCode::Char('a'), _) => Effect::Prepare(Prepare::Project),
            (KeyCode::Char('s'), Some(status)) => {
                Effect::Execute(Action::SyncProject(status.project.name))
            }
            (KeyCode::Char('r'), Some(status)) => self.confirm_vm(&status, VmChange::Restart),
            (KeyCode::Char('x'), Some(status)) => self.confirm_vm(&status, VmChange::Stop),
            (KeyCode::Char('s' | 'r' | 'x'), None) => Effect::None,
            _ => return None,
        };
        self.loading = None;
        Some(effect)
    }

    /// Ctrl shortcuts of the VM list. Plain keys filter the list.
    fn handle_vm_picker_key(&mut self, key: KeyEvent) -> Option<Effect> {
        let picker = self
            .picker()
            .filter(|picker| picker.tool == ToolId::SandboxVms)?;
        if !key.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        let effect = match key.code {
            KeyCode::Char('x') => match picker.selected_resource() {
                Some(Resource::Vm(vm)) => Effect::Prepare(Prepare::StopVm(vm.runtime_dir.clone())),
                _ => Effect::None,
            },
            KeyCode::Char('d') => Effect::Execute(Action::PruneVms),
            _ => return None,
        };
        self.loading = None;
        Some(effect)
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
            KeyCode::Char('x') => {
                let status = state.status.clone();
                return self.confirm_vm(&status, VmChange::Stop);
            }
            KeyCode::Char('r') => {
                let status = state.status.clone();
                return self.confirm_vm(&status, VmChange::Restart);
            }
            KeyCode::Char('p') => return Effect::Execute(Action::SetupProject(name)),
            KeyCode::Char('s') => return Effect::Execute(Action::SyncProject(name)),
            KeyCode::Char('v') => return Effect::Execute(Action::ReviewProject(name, worktree)),
            KeyCode::Char('a') => return Effect::Prepare(Prepare::ProjectWorktree(name)),
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
            KeyCode::Char('d') => {
                if let Some(worktree) = worktree {
                    self.confirm_remove_project_worktree(name, worktree);
                }
            }
            KeyCode::Esc => self.screen = Screen::Picker(state.return_picker.clone()),
            _ => {}
        }
        Effect::None
    }

    fn handle_tailor_key(&mut self, key: KeyEvent) -> Effect {
        let Screen::TailorFlake(state) = &self.screen else {
            return Effect::None;
        };
        let name = state.project.name.clone();
        match key.code {
            KeyCode::Char('a') => Effect::Execute(Action::TailorProject(
                name,
                Tailor::Agent(state.agent.clone()),
            )),
            KeyCode::Char('e') => Effect::Execute(Action::TailorProject(name, Tailor::Editor)),
            KeyCode::Char('s') | KeyCode::Esc => self.load_project(name),
            _ => Effect::None,
        }
    }

    fn handle_new_worktree_key(&mut self, key: KeyEvent) -> Effect {
        let Screen::NewWorktree(form) = &mut self.screen else {
            return Effect::None;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match (key.code, ctrl) {
            // Nothing to create without a name.
            (KeyCode::Enter, _) => return form.action().map_or(Effect::None, Effect::Execute),
            (KeyCode::Esc, _) => self.screen = (*form.back).clone(),
            (KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down, _) => {
                form.switch_focus()
            }
            (KeyCode::Left, _) if form.agents => form.cycle(-1),
            (KeyCode::Right, _) if form.agents => form.cycle(1),
            (KeyCode::Backspace, _) => {
                form.field().pop();
            }
            (KeyCode::Char('u'), true) => form.field().clear(),
            (KeyCode::Char('w'), true) => {
                // The words of a name end at `/` and `-` too.
                let separators: &[char] = match form.focus {
                    FormField::Name => &[' ', '/', '-'],
                    FormField::Prompt => &[' '],
                };
                let field = form.field();
                let kept = field
                    .trim_end_matches(separators)
                    .rfind(separators)
                    .map_or(0, |at| at + 1);
                field.truncate(kept);
            }
            (KeyCode::Char(ch), false) => form.field().push(ch),
            _ => {}
        }
        Effect::None
    }

    fn handle_project_form_key(&mut self, key: KeyEvent) -> Effect {
        let Screen::ProjectForm(form) = &mut self.screen else {
            return Effect::None;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let on_name = form.focus == ProjectField::Name;
        match (key.code, ctrl) {
            // Nothing to create without a name.
            (KeyCode::Enter, _) => return form.action().map_or(Effect::None, Effect::Execute),
            (KeyCode::Esc, _) => self.screen = (*form.back).clone(),
            (KeyCode::Tab | KeyCode::Down, _) => form.focus = form.focus.offset(1),
            (KeyCode::BackTab | KeyCode::Up, _) => form.focus = form.focus.offset(-1),
            (KeyCode::Left, _) => form.change(-1),
            (KeyCode::Right, _) => form.change(1),
            (KeyCode::Char(' '), false) if !on_name => form.change(1),
            (KeyCode::Backspace, _) if on_name => {
                form.name.pop();
            }
            (KeyCode::Char('u' | 'w'), true) if on_name => form.name.clear(),
            (KeyCode::Char(ch), false) if on_name => form.name.push(ch),
            _ => {}
        }
        Effect::None
    }

    /// Load the project list and open the project's details.
    fn load_project(&mut self, name: String) -> Effect {
        self.open_project = Some(name);
        self.screen = Screen::Picker(PickerState::new(ToolId::SandboxProjects));
        self.loading = Some(ToolId::SandboxProjects);
        Effect::Load(ToolId::SandboxProjects)
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> Effect {
        let Screen::Confirm(confirm) = &mut self.screen else {
            return Effect::None;
        };
        let count = confirm.choices.len() + 1;
        let picked = match key.code {
            KeyCode::Up => {
                confirm.scroll = confirm.scroll.saturating_sub(1);
                return Effect::None;
            }
            KeyCode::Down => {
                confirm.scroll = confirm.scroll.saturating_add(1);
                return Effect::None;
            }
            KeyCode::PageUp => {
                confirm.scroll = confirm.scroll.saturating_sub(10);
                return Effect::None;
            }
            KeyCode::PageDown => {
                confirm.scroll = confirm.scroll.saturating_add(10);
                return Effect::None;
            }
            KeyCode::Tab | KeyCode::Right => {
                confirm.selected = (confirm.selected + 1) % count;
                return Effect::None;
            }
            KeyCode::BackTab | KeyCode::Left => {
                confirm.selected = (confirm.selected + count - 1) % count;
                return Effect::None;
            }
            KeyCode::Char('n') | KeyCode::Esc => None,
            KeyCode::Enter => confirm
                .selected
                .checked_sub(1)
                .and_then(|index| confirm.choices.get(index)),
            KeyCode::Char(ch) => match confirm.choices.iter().find(|choice| choice.key == ch) {
                Some(choice) => Some(choice),
                None => return Effect::None,
            },
            _ => return Effect::None,
        };
        if let Some(choice) = picked {
            return Effect::Execute(choice.action.clone());
        }
        self.screen = (*confirm.back).clone();
        Effect::None
    }

    /// Ask before running a choice; Cancel returns to the current screen.
    pub(crate) fn confirm(&mut self, confirm: ConfirmState) {
        self.loading = None;
        let back = Box::new(self.screen.clone());
        self.screen = Screen::Confirm(ConfirmState { back, ..confirm });
    }

    fn confirm_cleanup(&mut self, worktree: Worktree) {
        let (title, branch) = if worktree.branch.is_empty() {
            (
                "Remove this worktree?",
                "Branch: (none; only the worktree will be removed)".to_string(),
            )
        } else {
            (
                "Remove this worktree and branch?",
                format!("Branch: {}", worktree.branch),
            )
        };
        let mut lines = vec![format!("Path: {}", worktree.path.display()), branch];
        if self.herdr {
            lines.push("Herdr: an open workspace for it is closed too".into());
        }
        let choices = vec![Choice::new('y', "Remove", Action::Cleanup(worktree))];
        self.confirm(ConfirmState::new(title, lines, choices));
    }

    /// Ask before stopping or restarting the project's running VM, which ends
    /// its agent sessions. A stopped VM has nothing to confirm.
    fn confirm_vm(&mut self, status: &ProjectStatus, change: VmChange) -> Effect {
        let name = status.project.name.clone();
        let Some(vm) = &status.vm else {
            return Effect::Execute(change.action(name));
        };
        let line = vm_line(vm, Some(status.worktrees.len()));
        let (title, label, effect) = match change {
            VmChange::Stop => (
                format!("Stop the VM of {name}?"),
                "Stop",
                "Its agent sessions end; worktrees and the guest home stay.",
            ),
            VmChange::Restart => (
                format!("Restart the VM of {name}?"),
                "Restart",
                "Its agent sessions end, then start again with the current worktrees.",
            ),
        };
        let choices = vec![Choice::new('y', label, change.action(name))];
        self.confirm(ConfirmState::new(title, vec![line, effect.into()], choices));
        Effect::None
    }

    /// Ask before removing a project worktree without saving its work, and
    /// whether to keep its branch.
    fn confirm_remove_project_worktree(&mut self, project: String, worktree: Worktree) {
        let lines = vec![
            format!("Path: {}", worktree.path.display()),
            format!("Branch: {}", branch_label(&worktree)),
            "Uncommitted changes and unsynced guest commits are lost.".into(),
            format!(
                "Saved agent-review/{}/* snapshots stay until prune; v still opens them.",
                worktree.name
            ),
        ];
        let title = format!("Remove {} without saving its work?", worktree.name);
        let choices = vec![
            Choice::new(
                'k',
                "Keep branch",
                Action::RemoveProjectWorktree(project.clone(), worktree.clone(), true),
            ),
            Choice::new(
                'd',
                "Delete branch",
                Action::RemoveProjectWorktree(project, worktree, false),
            ),
        ];
        self.confirm(ConfirmState::new(title, lines, choices));
    }

    /// Ask before stopping the VM with `runtime_dir`, described by `line`.
    pub(crate) fn confirm_vm_stop(&mut self, runtime_dir: PathBuf, line: String) {
        let choices = vec![Choice::new('y', "Stop", Action::StopVm(runtime_dir))];
        let lines = vec![
            line,
            "Its agent sessions end and its herdr machine is removed.".into(),
        ];
        self.confirm(ConfirmState::new("Stop this VM?", lines, choices));
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
                if let Some(name) = result.action.project_to_open() {
                    let name = name.to_string();
                    return self.load_project(name);
                }
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

fn worktree_label(worktree: &Worktree) -> String {
    format!(
        "{}  {}  [{}]",
        worktree.name,
        worktree.path.display(),
        branch_label(worktree)
    )
}

pub(crate) fn branch_label(worktree: &Worktree) -> &str {
    if worktree.branch.is_empty() {
        "(no branch)"
    } else {
        worktree.branch.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Action, App, Category, Effect, NewProject, NewWorktree, Prepare, Resource, Screen, Tailor,
        ToolId,
    };
    use crate::agent::Launch;
    use crate::agent::vm::ProjectStatus;
    use crate::aws::Profile;
    use crate::git::Worktree;
    use crate::kitty::Choice;
    use crate::sandbox::RunnerState;
    use crate::sandbox::project::{CreateOptions, SshKey};
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

    fn worktree(name: &str) -> Worktree {
        Worktree {
            name: name.into(),
            path: PathBuf::from(format!("/tmp/{name}")),
            branch: name.into(),
        }
    }

    /// Open the worktree manager with the given worktrees loaded.
    fn worktree_manager(worktrees: &[Worktree]) -> App {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('g')));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('1'))),
            Effect::Load(ToolId::GitWorktrees)
        );
        app.loaded(
            ToolId::GitWorktrees,
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
    fn review_remains_available_after_agent_worktrees_are_removed() {
        let mut app = App::new("/repo".into());
        app.loaded(
            ToolId::SandboxProjects,
            Ok(vec![Resource::Project(Box::new(project("demo", &[])))]),
        );
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('v'))),
            Effect::Execute(Action::ReviewProject("demo".into(), None))
        );
    }

    #[test]
    fn brackets_cycle_tabs_both_ways() {
        let mut app = App::new("/repo".into());
        for expected in [
            Category::Azure,
            Category::Git,
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
    fn git_tab_opens_the_worktree_manager() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('g')));
        assert_eq!(app.category, Category::Git);
        // Numbers beyond the category's actions do nothing.
        assert_eq!(app.handle_key(key(KeyCode::Char('2'))), Effect::None);
        assert_eq!(
            app.handle_key(key(KeyCode::Char('1'))),
            Effect::Load(ToolId::GitWorktrees)
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
        assert!(matches!(app.screen, Screen::Confirm(_)));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('y'))),
            Effect::Execute(Action::Cleanup(tree))
        );
    }

    fn project(name: &str, worktrees: &[Worktree]) -> ProjectStatus {
        ProjectStatus {
            project: crate::sandbox::project::Project {
                name: name.into(),
                dir: PathBuf::from(format!("/sandbox/projects/{name}")),
                repo: Some(PathBuf::from(format!("/p/{name}"))),
                state_dir: PathBuf::from(format!("/sandbox/state/{name}")),
                worktrees_root: PathBuf::from("/wt"),
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
            Ok(vec![Resource::Project(Box::new(project(
                "demo",
                &[first.clone(), second.clone()],
            )))]),
        );
        assert_eq!(app.handle_key(ctrl('a')), Effect::Prepare(Prepare::Project));
        for (ch, action) in [
            // The hovered project; its VM is stopped, so nothing to confirm.
            ('s', Action::SyncProject("demo".into())),
            ('r', Action::RestartProject("demo".into())),
            ('x', Action::StopProject("demo".into())),
        ] {
            assert_eq!(action.return_tool(), Some(ToolId::SandboxProjects));
            assert_eq!(app.handle_key(ctrl(ch)), Effect::Execute(action));
        }

        app.handle_key(key(KeyCode::Enter));
        assert!(matches!(app.screen, Screen::ProjectDetails(_)));
        for (ch, action) in [
            ('u', Action::StartProject("demo".into())),
            ('x', Action::StopProject("demo".into())),
            ('r', Action::RestartProject("demo".into())),
            ('p', Action::SetupProject("demo".into())),
            ('s', Action::SyncProject("demo".into())),
            (
                'v',
                Action::ReviewProject("demo".into(), Some(first.clone())),
            ),
            ('c', Action::EnterProjectWorktree(first.clone())),
        ] {
            assert_eq!(
                app.handle_key(key(KeyCode::Char(ch))),
                Effect::Execute(action)
            );
        }
        assert_eq!(
            app.handle_key(key(KeyCode::Char('a'))),
            Effect::Prepare(Prepare::ProjectWorktree("demo".into()))
        );
        // d asks whether to keep the branch.
        assert_eq!(app.handle_key(key(KeyCode::Char('d'))), Effect::None);
        let Screen::Confirm(confirm) = &app.screen else {
            panic!("expected a confirmation");
        };
        assert!(confirm.lines.iter().any(|line| line.contains("are lost")));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('k'))),
            Effect::Execute(Action::RemoveProjectWorktree(
                "demo".into(),
                first.clone(),
                true
            ))
        );
        // The action runs from the confirmation; cancel it to try the other.
        app.handle_key(key(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::ProjectDetails(_)));
        app.handle_key(key(KeyCode::Char('d')));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('d'))),
            Effect::Execute(Action::RemoveProjectWorktree(
                "demo".into(),
                first.clone(),
                false
            ))
        );
        app.handle_key(key(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::ProjectDetails(_)));
        app.handle_key(key(KeyCode::Down));
        let review = Action::ReviewProject("demo".into(), Some(second.clone()));
        assert_eq!(review.return_tool(), Some(ToolId::SandboxProjects));
        assert_eq!(review.project_to_open(), Some("demo"));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('v'))),
            Effect::Execute(review)
        );
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

    fn create_options() -> CreateOptions {
        CreateOptions {
            name: Some("demo".into()),
            repo: PathBuf::from("/p/demo"),
            template: Some("rust".into()),
            source: None,
            ssh_key: SshKey::Auto,
            proxy: true,
            backlog: false,
        }
    }

    #[test]
    fn new_projects_ask_for_their_options_in_a_form() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('s')));
        app.handle_key(key(KeyCode::Char('1')));
        app.loaded(ToolId::SandboxProjects, Ok(Vec::new()));
        assert_eq!(app.handle_key(ctrl('a')), Effect::Prepare(Prepare::Project));
        // Without a repository the list says why.
        app.project_form(Err("not a git repository".into()));
        assert!(matches!(
            &app.screen,
            Screen::Picker(picker) if picker.error.as_deref() == Some("not a git repository")
        ));

        app.project_form(Ok(create_options()));
        let Screen::ProjectForm(form) = &app.screen else {
            panic!("expected the project form");
        };
        assert_eq!((form.name.as_str(), form.template()), ("demo", "rust"));
        app.handle_key(ctrl('u'));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), Effect::None, "no name");
        for ch in "my-app?".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        app.handle_key(key(KeyCode::Backspace));
        assert!(!app.show_help, "? is name text");
        app.handle_key(key(KeyCode::Tab));
        app.handle_key(key(KeyCode::Right));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Char(' ')));
        app.handle_key(key(KeyCode::Tab));
        app.handle_key(key(KeyCode::Left));
        let want = CreateOptions {
            name: Some("my-app".into()),
            template: Some("go".into()),
            proxy: false,
            backlog: true,
            ..create_options()
        };
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::CreateProject(want))
        );
        app.handle_key(key(KeyCode::Esc));
        assert!(matches!(
            &app.screen,
            Screen::Picker(picker) if picker.tool == ToolId::SandboxProjects
        ));
    }

    #[test]
    fn new_projects_ask_how_to_tailor_the_flake_then_open_details() {
        let prompt = || {
            let mut app = App::new("/repo".into());
            app.tailor_prompt(
                NewProject {
                    name: "demo".into(),
                    summary: "Created demo".into(),
                },
                "claude".into(),
                "vi".into(),
            );
            app
        };
        let mut app = prompt();
        assert_eq!(
            app.handle_key(key(KeyCode::Char('a'))),
            Effect::Execute(Action::TailorProject(
                "demo".into(),
                Tailor::Agent("claude".into())
            ))
        );
        assert_eq!(
            app.handle_key(key(KeyCode::Char('e'))),
            Effect::Execute(Action::TailorProject("demo".into(), Tailor::Editor))
        );

        // Skipping, or leaving the result of tailoring, opens the project.
        assert_eq!(
            app.handle_key(key(KeyCode::Char('s'))),
            Effect::Load(ToolId::SandboxProjects)
        );
        let projects = || {
            Ok(vec![
                Resource::Project(Box::new(project("other", &[]))),
                Resource::Project(Box::new(project("demo", &[]))),
            ])
        };
        app.loaded(ToolId::SandboxProjects, projects());
        assert!(matches!(
            &app.screen,
            Screen::ProjectDetails(state) if state.status.project.name == "demo"
        ));

        let mut app = prompt();
        let action = Action::TailorProject("demo".into(), Tailor::Editor);
        app.finished(&action, "Done".into(), String::new(), false);
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Load(ToolId::SandboxProjects)
        );
        app.loaded(ToolId::SandboxProjects, projects());
        assert!(matches!(&app.screen, Screen::ProjectDetails(_)));
        // Later loads show the list again.
        app.loaded(ToolId::SandboxProjects, projects());
        assert!(matches!(&app.screen, Screen::Picker(_)));
    }

    #[test]
    fn stopping_or_restarting_a_running_vm_asks_first_and_names_it() {
        let mut app = App::new("/repo".into());
        let running = ProjectStatus {
            vm: Some(RunnerState {
                name: "agent-demo".into(),
                ssh_port: Some(2300),
                dir: "/sandbox/projects/demo".into(),
                ..RunnerState::default()
            }),
            ..project("demo", &[worktree("demo-agent-wt")])
        };
        app.loaded(
            ToolId::SandboxProjects,
            Ok(vec![Resource::Project(Box::new(running))]),
        );
        for (ch, action) in [
            ('x', Action::StopProject("demo".into())),
            ('r', Action::RestartProject("demo".into())),
        ] {
            // From the list with Ctrl, and from the details.
            for details in [false, true] {
                if details {
                    app.handle_key(key(KeyCode::Enter));
                    assert_eq!(app.handle_key(key(KeyCode::Char(ch))), Effect::None);
                } else {
                    assert_eq!(app.handle_key(ctrl(ch)), Effect::None);
                }
                let Screen::Confirm(confirm) = &app.screen else {
                    panic!("expected a confirmation");
                };
                assert_eq!(
                    confirm.lines[0],
                    "agent-demo: SSH port 2300, /sandbox/projects/demo, 1 worktree(s)"
                );
                assert_eq!(app.handle_key(key(KeyCode::Esc)), Effect::None);
                let back = if details { "details" } else { "list" };
                assert!(
                    matches!(
                        (&app.screen, details),
                        (Screen::ProjectDetails(_), true) | (Screen::Picker(_), false)
                    ),
                    "back to the {back}"
                );
                let confirmed = if details {
                    app.handle_key(key(KeyCode::Char(ch)))
                } else {
                    app.handle_key(ctrl(ch))
                };
                assert_eq!(confirmed, Effect::None);
                app.handle_key(key(KeyCode::Tab));
                assert_eq!(
                    app.handle_key(key(KeyCode::Enter)),
                    Effect::Execute(action.clone())
                );
                app.handle_key(key(KeyCode::Esc));
                if details {
                    app.handle_key(key(KeyCode::Esc));
                }
            }
        }
    }

    #[test]
    fn vm_list_stops_and_prunes() {
        let mut app = App::new("/repo".into());
        app.handle_key(key(KeyCode::Char('s')));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('2'))),
            Effect::Load(ToolId::SandboxVms)
        );
        let vm = |name: &str| RunnerState {
            name: format!("agent-{name}"),
            runtime_dir: format!("/state/vms/{name}").into(),
            ..RunnerState::default()
        };
        app.loaded(
            ToolId::SandboxVms,
            Ok(vec![
                Resource::Vm(Box::new(vm("a"))),
                Resource::Vm(Box::new(vm("b"))),
            ]),
        );
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Prepare(Prepare::StopVm("/state/vms/a".into()))
        );
        app.handle_key(key(KeyCode::Down));
        assert_eq!(
            app.handle_key(ctrl('x')),
            Effect::Prepare(Prepare::StopVm("/state/vms/b".into()))
        );
        // The runtime describes the VM; stopping it takes a confirmation.
        app.confirm_vm_stop("/state/vms/b".into(), "agent-b: SSH port 2301".into());
        let Screen::Confirm(confirm) = &app.screen else {
            panic!("expected a confirmation");
        };
        assert_eq!(confirm.lines[0], "agent-b: SSH port 2301");
        assert_eq!(app.handle_key(key(KeyCode::Enter)), Effect::None, "cancel");
        app.confirm_vm_stop("/state/vms/b".into(), "agent-b".into());
        assert_eq!(
            app.handle_key(key(KeyCode::Char('y'))),
            Effect::Execute(Action::StopVm("/state/vms/b".into()))
        );
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.handle_key(ctrl('d')), Effect::Execute(Action::PruneVms));
        assert_eq!(Action::PruneVms.return_tool(), Some(ToolId::SandboxVms));
    }

    #[test]
    fn manager_shortcuts_create_and_remove_with_cancel() {
        let tree = worktree("repo-agent-wt");
        let mut app = worktree_manager(std::slice::from_ref(&tree));
        // Without Herdr the form asks for the name only.
        assert_eq!(app.handle_key(ctrl('a')), Effect::None);
        assert!(matches!(&app.screen, Screen::NewWorktree(form) if !form.agents));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), Effect::None, "no name");
        app.handle_key(key(KeyCode::Tab));
        for ch in "feature/x".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::CreateWorktree("feature/x".into(), None))
        );
        app.handle_key(key(KeyCode::Esc));
        app.handle_key(ctrl('d'));
        assert!(matches!(app.screen, Screen::Confirm(_)));
        assert_eq!(app.handle_key(key(KeyCode::Char('n'))), Effect::None);
        assert!(matches!(
            &app.screen,
            Screen::Picker(picker) if picker.tool == ToolId::GitWorktrees
        ));
    }

    #[test]
    fn new_worktrees_ask_for_a_name_the_agent_and_prompt_with_herdr() {
        let mut app = worktree_manager(&[]);
        app.herdr = true;
        app.agent_kinds = vec!["pi".into(), "claude".into(), "codex".into()];
        app.default_agent = Some("claude".into());

        assert_eq!(app.handle_key(ctrl('a')), Effect::None);
        let Screen::NewWorktree(form) = &app.screen else {
            panic!("expected the new worktree form");
        };
        assert_eq!(form.target, NewWorktree::Local);
        assert_eq!(form.name, "");
        assert_eq!(form.kind(), Some("claude"));

        for ch in "fix/login".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        app.handle_key(ctrl('w'));
        for ch in "flaky".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        app.handle_key(key(KeyCode::Right));
        app.handle_key(key(KeyCode::Down));
        for ch in "Fix it?".chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
        app.handle_key(key(KeyCode::Backspace));
        assert!(!app.show_help, "? is prompt text");
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::CreateWorktree(
                "fix/flaky".into(),
                Some(Launch::new("codex", Some("Fix it".into())))
            ))
        );

        // Past the last kind comes no agent; Esc goes back to the list.
        app.handle_key(key(KeyCode::Right));
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::CreateWorktree("fix/flaky".into(), None))
        );
        // Back in the name, Ctrl-U clears it and Enter waits for one.
        app.handle_key(key(KeyCode::Up));
        app.handle_key(ctrl('u'));
        assert_eq!(app.handle_key(key(KeyCode::Enter)), Effect::None);
        app.handle_key(key(KeyCode::Esc));
        assert!(matches!(
            &app.screen,
            Screen::Picker(picker) if picker.tool == ToolId::GitWorktrees
        ));
    }

    #[test]
    fn project_worktrees_offer_only_the_agents_the_project_enables() {
        let mut app = App::new("/repo".into());
        app.herdr = true;
        app.agent_kinds = vec!["pi".into(), "claude".into(), "codex".into()];
        app.default_agent = Some("codex".into());
        app.handle_key(key(KeyCode::Char('s')));
        app.handle_key(key(KeyCode::Char('1')));
        app.loaded(
            ToolId::SandboxProjects,
            Ok(vec![Resource::Project(Box::new(project("demo", &[])))]),
        );
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(
            app.handle_key(key(KeyCode::Char('a'))),
            Effect::Prepare(Prepare::ProjectWorktree("demo".into()))
        );
        app.project_worktree_form(
            "demo".into(),
            "demo-agent-wt-2".into(),
            vec!["claude".into()],
            None,
        );
        let Screen::NewWorktree(form) = &app.screen else {
            panic!("expected the new worktree form");
        };
        assert_eq!(form.target, NewWorktree::Project("demo".into()));
        assert_eq!(form.name, "demo-agent-wt-2");
        // codex is not enabled, so nothing is chosen until picked.
        assert_eq!(form.kinds, ["claude"]);
        assert_eq!(form.kind(), None);
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::AddProjectWorktree(
                "demo".into(),
                "demo-agent-wt-2".into(),
                None
            ))
        );
        app.handle_key(key(KeyCode::Right));
        assert_eq!(
            app.handle_key(key(KeyCode::Enter)),
            Effect::Execute(Action::AddProjectWorktree(
                "demo".into(),
                "demo-agent-wt-2".into(),
                Some(Launch::new("claude", None))
            ))
        );
        app.handle_key(key(KeyCode::Esc));
        assert!(matches!(app.screen, Screen::ProjectDetails(_)));
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
            Action::CreateWorktree("a".into(), None),
            Action::EnterWorktree(worktree("a")),
            Action::Cleanup(worktree("a")),
        ] {
            app.finished(&action, "Done".into(), "done".into(), false);
            assert_eq!(
                app.handle_key(key(KeyCode::Esc)),
                Effect::Load(ToolId::GitWorktrees)
            );
        }
        for action in [
            Action::CreateProject(create_options()),
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
