//! Central sandbox projects: `~/.agent-sandbox/projects/<name>/` holds a
//! project's flake, out of the agent's reach. The project's one VM mounts
//! every agent worktree of its repository at `/workspace/<worktree>`; the
//! list comes from the generated `worktrees.nix` next to the flake.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::setup;
use crate::git::{self, Worktree};
use crate::{Error, exec};

/// Overrides the central sandbox directory, `~/.agent-sandbox`.
pub(crate) const DIR_ENV: &str = "CLI_TOOLBOX_SANDBOX_DIR";

/// Overrides the agent-sandbox flake that templates and inputs come from.
pub(crate) const SOURCE_ENV: &str = "CLI_TOOLBOX_AGENT_SANDBOX_FLAKE";
const DEFAULT_SOURCE: &str = "git+ssh://git@github.com/vvartiainen/agent-sandbox-setup";

pub(crate) const TEMPLATES: [&str; 3] = ["nodejs", "rust", "go"];

/// The placeholder key in agent-sandbox templates from before the runner
/// authorized its own key.
const KEY_PLACEHOLDER: &str = "ssh-ed25519 AAAA... your-key-comment";

/// Instructions for an agent that tailors the flake, in the agent-sandbox repo.
const SETUP_PROMPT: &str = "docs/setup-agent.md";

const WORKTREES_FILE: &str = "worktrees.nix";
const PROJECT_FILE: &str = "project.json";
/// The worktrees the running VM was started with, in the state directory.
const MOUNTED_FILE: &str = "mounted-worktrees.nix";

/// A project directory under `<sandbox dir>/projects`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Project {
    pub(crate) name: String,
    /// The flake directory; the VM is started from here.
    pub(crate) dir: PathBuf,
    /// The main checkout whose agent worktrees the VM mounts. `None` for a
    /// flake written by hand.
    pub(crate) repo: Option<PathBuf>,
    /// `home.img` and the runner's logs.
    pub(crate) state_dir: PathBuf,
    /// Where the repository's project worktrees are grouped:
    /// `<worktrees_root>/<repo>/<worktree>`, see [`git::worktrees_root`].
    pub(crate) worktrees_root: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Metadata {
    repo: PathBuf,
}

impl Project {
    fn at(root: &Path, name: &str) -> Self {
        let dir = root.join("projects").join(name);
        let repo = fs::read_to_string(dir.join(PROJECT_FILE))
            .ok()
            .and_then(|text| serde_json::from_str::<Metadata>(&text).ok())
            .map(|metadata| metadata.repo);
        Self {
            name: name.into(),
            dir,
            repo,
            state_dir: root.join("state").join(name),
            worktrees_root: git::worktrees_root().unwrap_or_else(|_| root.join("worktrees")),
        }
    }

    /// The repository, for commands that manage its worktrees.
    pub(crate) fn repo(&self) -> Result<&Path, Error> {
        self.repo.as_deref().ok_or_else(|| {
            Error::msg(format!(
                "project {} has no {PROJECT_FILE} naming its repository",
                self.name
            ))
        })
    }

    /// The repository's project worktrees, the ones the VM mounts.
    pub(crate) fn worktrees(&self) -> Result<Vec<Worktree>, Error> {
        git::project_worktrees(self.repo()?, &self.worktrees_root)
    }

    /// Whether `worktrees.nix` changed since the running VM was started.
    pub(crate) fn restart_needed(&self) -> bool {
        let read = |path: PathBuf| fs::read_to_string(path).unwrap_or_default();
        self.repo.is_some()
            && read(self.dir.join(WORKTREES_FILE)) != read(self.state_dir.join(MOUNTED_FILE))
    }

    /// Whether the running VM mounts the worktree `name`: it was recorded
    /// when the VM started. Unknown without a record, which counts as yes.
    pub(crate) fn mounts(&self, name: &str) -> bool {
        match fs::read_to_string(self.state_dir.join(MOUNTED_FILE)) {
            Ok(text) => text.contains(&format!("  {} = ", nix_string(name))),
            Err(_) => true,
        }
    }

    /// Remember the worktrees the VM was just started with.
    pub(crate) fn record_mounted(&self) -> Result<(), Error> {
        if self.repo.is_none() {
            return Ok(());
        }
        fs::create_dir_all(&self.state_dir)?;
        let text = fs::read_to_string(self.dir.join(WORKTREES_FILE)).unwrap_or_default();
        fs::write(self.state_dir.join(MOUNTED_FILE), text)?;
        Ok(())
    }

    /// Rewrite `worktrees.nix` for `worktrees`. Returns whether it changed.
    pub(crate) fn write_worktrees(&self, worktrees: &[Worktree]) -> Result<bool, Error> {
        let path = self.dir.join(WORKTREES_FILE);
        let text = worktrees_nix(worktrees);
        if fs::read_to_string(&path).is_ok_and(|old| old == text) {
            return Ok(false);
        }
        fs::write(&path, text)
            .map_err(|err| Error::msg(format!("write {}: {err}", path.display())))?;
        Ok(true)
    }
}

/// The central sandbox directory: `$CLI_TOOLBOX_SANDBOX_DIR`, or
/// `~/.agent-sandbox`.
pub(crate) fn sandbox_dir() -> Result<PathBuf, Error> {
    if let Some(dir) = std::env::var_os(DIR_ENV).filter(|dir| !dir.is_empty()) {
        return std::path::absolute(&dir)
            .map_err(|err| Error::msg(format!("resolve {DIR_ENV}: {err}")));
    }
    dirs::home_dir()
        .map(|home| home.join(".agent-sandbox"))
        .ok_or_else(|| Error::msg("resolve home directory: home directory not found"))
}

/// Every project with a `flake.nix`, sorted by name.
pub(crate) fn projects(root: &Path) -> Result<Vec<Project>, Error> {
    let dir = root.join("projects");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(Error::msg(format!("read {}: {err}", dir.display()))),
    };
    let mut projects: Vec<Project> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_str()?.to_string();
            entry
                .path()
                .join("flake.nix")
                .is_file()
                .then(|| Project::at(root, &name))
        })
        .collect();
    projects.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(projects)
}

pub(crate) fn load(root: &Path, name: &str) -> Result<Project, Error> {
    projects(root)?
        .into_iter()
        .find(|project| project.name == name)
        .ok_or_else(|| {
            Error::msg(format!(
                "no sandbox project \"{name}\" in {}",
                root.join("projects").display()
            ))
        })
}

/// The project named `name`, else the one for `repo`, else the only one.
pub(crate) fn resolve(
    root: &Path,
    name: Option<&str>,
    repo: Option<&Path>,
) -> Result<Project, Error> {
    if let Some(name) = name {
        return load(root, name);
    }
    let projects = projects(root)?;
    if let Some(repo) = repo
        && let Some(project) = projects
            .iter()
            .find(|project| project.repo.as_deref() == Some(repo))
    {
        return Ok(project.clone());
    }
    match projects.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(Error::msg(
            "no sandbox projects yet; create one with `cli-toolbox agent project create`",
        )),
        _ => Err(Error::msg(
            "several sandbox projects; name one (see `cli-toolbox agent project list`)",
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CreateOptions {
    /// Defaults to the repository's directory name.
    pub(crate) name: Option<String>,
    /// The main checkout of the repository.
    pub(crate) repo: PathBuf,
    /// Defaults to one matching the repository's files.
    pub(crate) template: Option<String>,
    /// The agent-sandbox flake reference.
    pub(crate) source: Option<String>,
    /// An extra public key to authorize. The runner authorizes its own key
    /// for every Mac, so a project needs none.
    pub(crate) ssh_key: Option<PathBuf>,
    /// Take agent credentials from agent-proxy on the host.
    pub(crate) proxy: bool,
    /// Give the project a backlog under [`setup::backlog_root`], if there is one.
    pub(crate) backlog: bool,
}

/// Create `<root>/projects/<name>/flake.nix` from an agent-sandbox template.
/// `worktrees.nix` starts empty; the VM needs at least one worktree.
pub(crate) fn create(
    root: &Path,
    options: &CreateOptions,
    stderr: &mut dyn Write,
) -> Result<Project, Error> {
    let name = match &options.name {
        Some(name) => valid_name(name)?,
        None => project_name(&options.repo),
    };
    let template = options
        .template
        .clone()
        .unwrap_or_else(|| detect_template(&options.repo).into());
    if !TEMPLATES.contains(&template.as_str()) {
        return Err(Error::msg(format!(
            "unknown template \"{template}\"; use one of {}",
            TEMPLATES.join(", ")
        )));
    }
    let mut project = Project::at(root, &name);
    let flake = project.dir.join("flake.nix");
    if flake.exists() {
        return Err(Error::msg(format!("{} already exists", flake.display())));
    }
    let port = free_port(&used_ports(root))?;
    fs::create_dir_all(&project.dir)
        .map_err(|err| Error::msg(format!("create {}: {err}", project.dir.display())))?;
    let source = source(options.source.as_deref());
    let reference = format!("{source}#{template}");
    writeln!(stderr, "Creating {} from {reference}", flake.display())?;
    exec::run_on_stderr_in(
        "nix",
        &["flake", "init", "-t", &reference],
        Some(&project.dir),
        &[],
    )?;

    let key = options
        .ssh_key
        .as_deref()
        .map(read_public_key)
        .transpose()?;
    let text = fs::read_to_string(&flake)?;
    let backlog_dir = options
        .backlog
        .then(setup::backlog_root)
        .flatten()
        .map(|root| root.join(&name));
    let settings = FlakeSettings {
        source: &source,
        name: &name,
        state_dir: &project.state_dir,
        ssh_port: port,
        key: key.as_deref(),
        proxy: options.proxy,
        backlog_dir: backlog_dir.as_deref(),
    };
    fs::write(&flake, customize_flake(&text, &settings))?;
    project.write_worktrees(&[])?;
    let metadata = Metadata {
        repo: options.repo.clone(),
    };
    fs::write(
        project.dir.join(PROJECT_FILE),
        serde_json::to_string_pretty(&metadata).map_err(|err| Error::msg(err.to_string()))? + "\n",
    )?;
    project.repo = Some(options.repo.clone());
    track_in_git(&project.dir, &["flake.nix", WORKTREES_FILE, PROJECT_FILE]);
    Ok(project)
}

/// Nix only sees files git tracks when the sandbox directory is a repository.
fn track_in_git(dir: &Path, files: &[&str]) {
    let inside = exec::combined_output("git", &["rev-parse", "--is-inside-work-tree"], Some(dir))
        .is_ok_and(|out| out.trim() == "true");
    if inside {
        let mut args = vec!["add", "--intent-to-add", "--"];
        args.extend(files);
        let _ = exec::combined_output("git", &args, Some(dir));
    }
}

/// The agent-sandbox flake: `given`, `$CLI_TOOLBOX_AGENT_SANDBOX_FLAKE`, then
/// the upstream repository.
pub(crate) fn source(given: Option<&str>) -> String {
    given
        .map(str::to_string)
        .or_else(|| {
            std::env::var(SOURCE_ENV)
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_SOURCE.into())
}

struct FlakeSettings<'a> {
    source: &'a str,
    name: &'a str,
    state_dir: &'a Path,
    ssh_port: u16,
    key: Option<&'a str>,
    proxy: bool,
    backlog_dir: Option<&'a Path>,
}

/// Point a template at `source` and the project's state, mount the
/// generated worktrees, and apply guest paths and init steps to every
/// worktree. Those name no worktree, so adding or removing one does not
/// rebuild the guest system. Adds the cli-toolbox defaults module and the
/// backlog folder.
fn customize_flake(text: &str, settings: &FlakeSettings) -> String {
    let mut named = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|line| {
            let indent = &line[..line.len() - line.trim_start().len()];
            let trimmed = line.trim_start();
            if trimmed.starts_with("inputs.agent-sandbox.url =") {
                format!(
                    "{indent}inputs.agent-sandbox.url = {};",
                    nix_string(settings.source)
                )
            } else if !named && trimmed.starts_with("name = \"") {
                named = true;
                format!("{indent}name = {};", nix_string(settings.name))
            } else if trimmed.starts_with("workspaces.app =") {
                format!(
                    "{indent}# One workspace per agent worktree, written by cli-toolbox.\n{indent}workspaces = import ./{WORKTREES_FILE};"
                )
            } else if trimmed.starts_with("stateDir =") {
                format!(
                    "{indent}stateDir = {};",
                    nix_string(&settings.state_dir.to_string_lossy())
                )
            } else if trimmed.starts_with("ssh.hostPort =") {
                let port = format!("{indent}ssh.hostPort = {};", settings.ssh_port);
                match settings.backlog_dir {
                    Some(dir) => format!(
                        "{port}\n{indent}backlogDir = {};",
                        nix_string(&dir.to_string_lossy())
                    ),
                    None => port,
                }
            } else if trimmed == "environment = [" {
                let module = setup::defaults_module(
                    settings.name,
                    settings.proxy,
                    settings.backlog_dir.is_some(),
                    &format!("{indent}  "),
                );
                format!("{line}\n{module}")
            } else if let Some(paths) = per_worktree_guest_paths(trimmed) {
                format!("{indent}{paths}")
            } else if trimmed.starts_with("init.steps.") && trimmed.ends_with("= {") {
                format!("{line}\n{indent}  eachWorkspace = true;")
            } else if let (Some(key), "ssh.enable = true;") = (settings.key, trimmed) {
                format!(
                    "{line}\n{indent}ssh.authorizedKeys = [ {} ];",
                    nix_string(key)
                )
            } else {
                line.to_string()
            }
        })
        .collect();
    if text.ends_with('\n') {
        out.push(String::new());
    }
    let joined = out.join("\n");
    match settings.key {
        Some(key) => joined.replace(KEY_PLACEHOLDER, key),
        None => joined,
    }
}

/// `guestPaths = [ "app/target" ];` → `"*/target"`, the path in every worktree.
fn per_worktree_guest_paths(line: &str) -> Option<String> {
    let list = line
        .strip_prefix("guestPaths = [")?
        .strip_suffix("];")?
        .trim();
    let paths: Vec<&str> = list
        .split('"')
        .skip(1)
        .step_by(2)
        .map(|path| path.strip_prefix("app/"))
        .collect::<Option<_>>()?;
    if paths.is_empty() {
        return None;
    }
    let items: Vec<String> = paths.iter().map(|path| format!("\"*/{path}\"")).collect();
    Some(format!("guestPaths = [ {} ];", items.join(" ")))
}

/// The generated `worktrees.nix`: worktree name → host path.
pub(crate) fn worktrees_nix(worktrees: &[Worktree]) -> String {
    let mut text = String::from(
        "# Written by cli-toolbox from the project's agent worktrees; do not edit.\n{\n",
    );
    for worktree in worktrees {
        text.push_str(&format!(
            "  {} = {};\n",
            nix_string(&worktree.name),
            nix_string(&worktree.path.to_string_lossy())
        ));
    }
    text.push_str("}\n");
    text
}

pub(crate) fn nix_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace("${", "\\${");
    format!("\"{escaped}\"")
}

/// SSH host ports taken by the projects' flakes.
fn used_ports(root: &Path) -> Vec<u16> {
    projects(root)
        .unwrap_or_default()
        .iter()
        .filter_map(|project| fs::read_to_string(project.dir.join("flake.nix")).ok())
        .flat_map(|text| {
            text.lines()
                .filter_map(|line| {
                    line.trim()
                        .strip_prefix("ssh.hostPort =")?
                        .trim()
                        .trim_end_matches(';')
                        .parse()
                        .ok()
                })
                .collect::<Vec<u16>>()
        })
        .collect()
}

fn free_port(used: &[u16]) -> Result<u16, Error> {
    pick_port(used, |port| {
        std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
    })
}

/// The first SSH port that no project uses and `bindable` accepts.
fn pick_port(used: &[u16], bindable: impl Fn(u16) -> bool) -> Result<u16, Error> {
    let mut ports = super::SSH_PORTS;
    ports
        .find(|port| !used.contains(port) && bindable(*port))
        .ok_or_else(|| {
            Error::msg(format!(
                "no free SSH port in {}-{}",
                super::SSH_PORTS.start(),
                super::SSH_PORTS.end()
            ))
        })
}

pub(crate) fn detect_template(repo: &Path) -> &'static str {
    if repo.join("Cargo.toml").is_file() {
        "rust"
    } else if repo.join("go.mod").is_file() {
        "go"
    } else {
        "nodejs"
    }
}

fn valid_name(name: &str) -> Result<String, Error> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-');
    if ok {
        Ok(name.into())
    } else {
        Err(Error::msg(format!(
            "project name \"{name}\" must use lower case letters, digits and dashes"
        )))
    }
}

/// A project name from the checkout directory: lower case letters, digits
/// and dashes, as the guest host name `agent-<name>` needs.
pub(crate) fn project_name(repo: &Path) -> String {
    let raw = repo
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let name = name.trim_matches('-');
    if name.is_empty() {
        "project".into()
    } else {
        name.into()
    }
}

fn read_public_key(path: &Path) -> Result<String, Error> {
    let text = fs::read_to_string(path)
        .map_err(|err| Error::msg(format!("read {}: {err}", path.display())))?;
    let key = text.lines().next().unwrap_or_default().trim();
    if key.starts_with("ssh-") || key.starts_with("ecdsa-") {
        Ok(key.to_string())
    } else {
        Err(Error::msg(format!(
            "{} is not an SSH public key",
            path.display()
        )))
    }
}

#[derive(Deserialize)]
struct FlakeMetadata {
    path: PathBuf,
}

/// The store path of the agent-sandbox flake `source`.
pub(crate) fn source_dir(source: &str) -> Result<PathBuf, Error> {
    let json = exec::combined_output("nix", &["flake", "metadata", "--json", source], None)?;
    let metadata: FlakeMetadata = serde_json::from_str(json.lines().last().unwrap_or_default())
        .map_err(|err| Error::msg(format!("parse nix flake metadata: {err}")))?;
    Ok(metadata.path)
}

/// Start `agent` in the repository with agent-sandbox's setup instructions
/// to tailor the project's flake.
pub(crate) fn run_setup_agent(
    project: &Project,
    source: Option<&str>,
    agent: &str,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let source = self::source(source);
    let repo = project.repo()?;
    let instructions = fs::read_to_string(source_dir(&source)?.join(SETUP_PROMPT))
        .map_err(|err| Error::msg(format!("read {SETUP_PROMPT} from {source}: {err}")))?;
    let prompt = format!(
        "{instructions}\n\nThe flake to tailor is {}/flake.nix, kept centrally. It is shared by every agent worktree of this repository: `vm.workspaces` comes from the generated ./{WORKTREES_FILE} (worktree name → path, mounted at /workspace/<name>), so write guest paths and init steps for every worktree without naming any: `guestPaths = [ \"*/target\" ];` and init steps with `eachWorkspace = true;` (their `directory` is then relative to each worktree). The guest system then stays the same when worktrees come and go. Do not edit {WORKTREES_FILE} or use it outside `vm.workspaces`. Leave the cli-toolbox defaults module (proxy, ai) and `vm.backlogDir` as they are; cli-toolbox sets up the host side of both. The flake may already be tailored: then update it for what changed in the repository rather than starting over.",
        project.dir.display()
    );
    exec::require(agent)?;
    writeln!(stderr, "Starting {agent} to tailor the sandbox flake")?;
    exec::run_on_stderr_in(agent, &[&prompt], Some(repo), &[])
}

/// Update the project's flake inputs with `nix flake update`, whose output
/// streams to stderr, and let git track the new `flake.lock`.
pub(crate) fn update_flake(project: &Project) -> Result<(), Error> {
    exec::run_on_stderr_in("nix", &["flake", "update"], Some(&project.dir), &[])?;
    track_in_git(&project.dir, &["flake.lock"]);
    Ok(())
}

/// The editor command: `$VISUAL`, then `$EDITOR`, then `vi`, split into
/// words so values such as `code -w` work.
pub(crate) fn editor() -> Vec<String> {
    editor_from(std::env::var("VISUAL").ok(), std::env::var("EDITOR").ok())
}

fn editor_from(visual: Option<String>, editor: Option<String>) -> Vec<String> {
    [visual, editor]
        .into_iter()
        .flatten()
        .map(|value| {
            value
                .split_whitespace()
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .find(|words| !words.is_empty())
        .unwrap_or_else(|| vec!["vi".into()])
}

/// Open the project's flake in the editor, in the project directory.
pub(crate) fn edit_flake(project: &Project) -> Result<(), Error> {
    let editor = editor();
    let flake = project.dir.join("flake.nix");
    let mut args: Vec<&str> = editor[1..].iter().map(String::as_str).collect();
    let flake = flake.to_string_lossy();
    args.push(&flake);
    exec::run_on_stderr_in(&editor[0], &args, Some(&project.dir), &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_prefers_visual_then_editor_then_vi() {
        let some = |value: &str| Some(value.to_string());
        assert_eq!(editor_from(some("code -w"), some("nvim")), ["code", "-w"]);
        assert_eq!(editor_from(some(" "), some("nvim")), ["nvim"]);
        assert_eq!(editor_from(None, None), ["vi"]);
    }

    const TEMPLATE: &str = r#"{
  inputs.agent-sandbox.url = "path:/Users/you/prog/agent-sandbox";
  outputs = { agent-sandbox, ... }:
    agent-sandbox.lib.microvm.mkFlake {
      name = "my-rust-project";
      environment = [
        ({ pkgs, ... }: {
          guestPaths = [ "app/target" "app/.venv" ];
          init.steps.cargo-fetch = {
            script = "cargo fetch";
          };
          ssh.enable = true;
          name = "other";
        })
      ];
      vm = {
        workspaces.app = "/Users/you/prog/my-rust-project";
        stateDir = "/Users/you/.agent-sandbox/state/my-rust-project";
        ssh.hostPort = 2222;
      };
    };
}
"#;

    fn settings<'a>(state_dir: &'a Path, key: Option<&'a str>) -> FlakeSettings<'a> {
        FlakeSettings {
            source: "github:me/agent-sandbox",
            name: "cli-toolbox",
            state_dir,
            ssh_port: 2305,
            key,
            proxy: true,
            backlog_dir: Some(Path::new("/home/me/Agents/cli-toolbox")),
        }
    }

    #[test]
    fn customizes_the_template_for_worktrees() {
        let state = Path::new("/home/me/.agent-sandbox/state/cli-toolbox");
        let text = customize_flake(
            TEMPLATE,
            &settings(state, Some("ssh-ed25519 AAAAC3 me@mac")),
        );
        for want in [
            "  inputs.agent-sandbox.url = \"github:me/agent-sandbox\";",
            "      name = \"cli-toolbox\";",
            "        workspaces = import ./worktrees.nix;",
            "        stateDir = \"/home/me/.agent-sandbox/state/cli-toolbox\";",
            "        ssh.hostPort = 2305;",
            "          guestPaths = [ \"*/target\" \"*/.venv\" ];",
            "          init.steps.cargo-fetch = {\n            eachWorkspace = true;\n",
            "          ssh.enable = true;\n          ssh.authorizedKeys = [ \"ssh-ed25519 AAAAC3 me@mac\" ];",
            "name = \"other\"",
            "      environment = [\n        # cli-toolbox defaults",
            "          proxy = {\n            enable = true;\n            client = \"cli-toolbox\";",
            "            backlog.enable = true;",
            "        ssh.hostPort = 2305;\n        backlogDir = \"/home/me/Agents/cli-toolbox\";",
        ] {
            assert!(text.contains(want), "missing {want}:\n{text}");
        }
        assert!(text.ends_with("}\n"));
        // The runner authorizes its own key; no project needs one.
        assert!(!customize_flake(TEMPLATE, &settings(state, None)).contains("authorizedKeys"));
    }

    #[test]
    fn an_extra_key_replaces_an_older_template_placeholder() {
        let state = Path::new("/home/me/.agent-sandbox/state/cli-toolbox");
        let older = TEMPLATE.replace(
            "ssh.enable = true;",
            &format!("ssh.authorizedKeys = [ \"{KEY_PLACEHOLDER}\" ];"),
        );
        let text = customize_flake(&older, &settings(state, Some("ssh-ed25519 AAAAC3 me@mac")));
        assert!(text.contains("ssh.authorizedKeys = [ \"ssh-ed25519 AAAAC3 me@mac\" ];"));
        assert!(!text.contains(KEY_PLACEHOLDER));
    }

    #[test]
    fn guest_paths_outside_app_are_left_alone() {
        assert_eq!(
            per_worktree_guest_paths("guestPaths = [ \"lib/x\" ];"),
            None
        );
        assert_eq!(per_worktree_guest_paths("guestPaths = [ ];"), None);
    }

    #[test]
    fn worktrees_nix_maps_names_to_paths() {
        let worktrees = [Worktree {
            name: "demo-agent-wt".into(),
            path: "/wt/demo/demo-agent-wt".into(),
            branch: "demo-agent-wt".into(),
        }];
        let text = worktrees_nix(&worktrees);
        assert!(
            text.contains("  \"demo-agent-wt\" = \"/wt/demo/demo-agent-wt\";\n"),
            "{text}"
        );
        assert!(worktrees_nix(&[]).ends_with("{\n}\n"));
        assert_eq!(nix_string("a\"b${c}\\"), r#""a\"b\${c}\\""#);
    }

    #[test]
    fn discovers_projects_and_resolves_by_repo() {
        let root = tempfile::tempdir().unwrap();
        assert!(projects(root.path()).unwrap().is_empty());
        for (name, repo) in [("beta", "/p/beta"), ("alpha", "/p/alpha")] {
            let dir = root.path().join("projects").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("flake.nix"), "{ }").unwrap();
            fs::write(dir.join(PROJECT_FILE), format!("{{\"repo\": \"{repo}\"}}")).unwrap();
        }
        // A directory without a flake is not a project.
        fs::create_dir_all(root.path().join("projects/scratch")).unwrap();

        let found = projects(root.path()).unwrap();
        let names: Vec<_> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["alpha", "beta"]);
        assert_eq!(found[0].repo.as_deref(), Some(Path::new("/p/alpha")));
        assert_eq!(found[0].state_dir, root.path().join("state/alpha"));

        let by_repo = resolve(root.path(), None, Some(Path::new("/p/beta"))).unwrap();
        assert_eq!(by_repo.name, "beta");
        assert!(resolve(root.path(), None, None).is_err());
        assert_eq!(
            resolve(root.path(), Some("alpha"), None).unwrap().name,
            "alpha"
        );
        assert!(resolve(root.path(), Some("gamma"), None).is_err());
    }

    #[test]
    fn restart_is_needed_when_worktrees_change() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("projects/demo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("flake.nix"), "{ }").unwrap();
        fs::write(dir.join(PROJECT_FILE), r#"{"repo": "/p/demo"}"#).unwrap();
        let project = load(root.path(), "demo").unwrap();

        assert!(project.write_worktrees(&[]).unwrap());
        assert!(!project.write_worktrees(&[]).unwrap());
        project.record_mounted().unwrap();
        assert!(!project.restart_needed());
        let worktree = Worktree {
            name: "demo-agent-wt".into(),
            path: "/wt/demo-agent-wt".into(),
            branch: "demo-agent-wt".into(),
        };
        assert!(!project.mounts("demo-agent-wt"));
        assert!(project.write_worktrees(&[worktree]).unwrap());
        assert!(project.restart_needed());
        project.record_mounted().unwrap();
        assert!(project.mounts("demo-agent-wt"));
        assert!(!project.mounts("demo-agent-wt-2"));
    }

    #[test]
    fn ports_in_project_flakes_are_used() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("projects/demo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("flake.nix"),
            "vm = {\n  ssh.hostPort = 2300;\n};\n",
        )
        .unwrap();
        assert_eq!(used_ports(root.path()), [2300]);
        assert_eq!(pick_port(&[2300], |_| true).unwrap(), 2301);
        assert_eq!(pick_port(&[2300], |port| port > 2310).unwrap(), 2311);
        assert!(pick_port(&[], |_| false).is_err());
    }

    #[test]
    fn templates_follow_the_repository() {
        let repo = tempfile::tempdir().unwrap();
        assert_eq!(detect_template(repo.path()), "nodejs");
        fs::write(repo.path().join("go.mod"), "module x\n").unwrap();
        assert_eq!(detect_template(repo.path()), "go");
        fs::write(repo.path().join("Cargo.toml"), "[package]\n").unwrap();
        assert_eq!(detect_template(repo.path()), "rust");
    }

    #[test]
    fn project_names_are_host_name_safe() {
        assert_eq!(project_name(Path::new("/p/My_App.v2")), "my-app-v2");
        assert_eq!(project_name(Path::new("/p/cli-toolbox")), "cli-toolbox");
        assert_eq!(project_name(Path::new("/")), "project");
        assert!(valid_name("my-app").is_ok());
        assert!(valid_name("My App").is_err());
        assert!(valid_name("").is_err());
    }

    #[test]
    fn reads_public_keys_only() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id.pub");
        fs::write(&key, "ssh-ed25519 AAAAC3 me@mac\n").unwrap();
        assert_eq!(read_public_key(&key).unwrap(), "ssh-ed25519 AAAAC3 me@mac");
        fs::write(&key, "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
        assert!(read_public_key(&key).is_err());
    }
}
