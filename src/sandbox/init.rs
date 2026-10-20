//! `agent sandbox init`: create a project's sandbox flake from an
//! agent-sandbox template, and optionally let a coding agent tailor it.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{Error, exec, git};

/// Overrides the agent-sandbox flake that templates and inputs come from.
pub(crate) const SOURCE_ENV: &str = "CLI_TOOLBOX_AGENT_SANDBOX_FLAKE";
const DEFAULT_SOURCE: &str = "git+ssh://git@github.com/vvartiainen/agent-sandbox-setup";

/// The placeholder key in the agent-sandbox templates.
const KEY_PLACEHOLDER: &str = "ssh-ed25519 AAAA... your-key-comment";

/// Instructions for an agent that tailors the flake, in the agent-sandbox repo.
const SETUP_PROMPT: &str = "SETUP-AGENT.md";

pub(crate) const TEMPLATES: [&str; 3] = ["nodejs", "rust", "go"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SshKey {
    /// `~/.ssh/*.pub`, then the first key of the SSH agent.
    Auto,
    File(PathBuf),
    None,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InitOptions {
    pub(crate) template: String,
    /// Relative to the main checkout: `sandbox` or `.`.
    pub(crate) dir: PathBuf,
    /// The agent-sandbox flake reference.
    pub(crate) source: Option<String>,
    pub(crate) ssh_key: SshKey,
    /// Start this agent kind on the host with the setup instructions.
    pub(crate) ai: Option<String>,
}

/// Create `<main checkout>/<dir>/flake.nix` from a template.
pub(crate) fn init(
    cwd: &Path,
    options: &InitOptions,
    stderr: &mut dyn Write,
) -> Result<PathBuf, Error> {
    if !TEMPLATES.contains(&options.template.as_str()) {
        return Err(Error::msg(format!(
            "unknown template \"{}\"; use one of {}",
            options.template,
            TEMPLATES.join(", ")
        )));
    }
    let main = git::main_checkout(cwd)?;
    let dir = main.join(&options.dir);
    let flake = dir.join("flake.nix");
    if flake.exists() {
        return Err(Error::msg(format!("{} already exists", flake.display())));
    }
    fs::create_dir_all(&dir)
        .map_err(|err| Error::msg(format!("create {}: {err}", dir.display())))?;
    let source = options
        .source
        .clone()
        .or_else(|| {
            std::env::var(SOURCE_ENV)
                .ok()
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_SOURCE.into());
    let template = format!("{source}#{}", options.template);
    writeln!(stderr, "Creating {} from {template}", flake.display())?;
    exec::run_on_stderr_in("nix", &["flake", "init", "-t", &template], Some(&dir), &[])?;

    let key = match &options.ssh_key {
        SshKey::Auto => find_public_key(),
        SshKey::File(path) => Some(read_public_key(path)?),
        SshKey::None => None,
    };
    if key.is_none() && options.ssh_key == SshKey::Auto {
        writeln!(
            stderr,
            "No SSH public key found; replace the placeholder in authorizedKeys before starting the VM."
        )?;
    }
    let name = project_name(&main);
    let text = fs::read_to_string(&flake)?;
    fs::write(
        &flake,
        customize_flake(&text, &source, &name, key.as_deref()),
    )?;

    if !git::is_ignored(&main, ".agent-sandbox/") {
        append_line(&main.join(".gitignore"), ".agent-sandbox/")?;
        writeln!(stderr, "Added .agent-sandbox/ to .gitignore")?;
    }
    // Nix only sees files git tracks; commit the flake when it is ready.
    let relative = flake.strip_prefix(&main).unwrap_or(&flake);
    exec::combined_output(
        "git",
        &["add", "--intent-to-add", "--", &relative.to_string_lossy()],
        Some(&main),
    )?;

    if let Some(agent) = &options.ai {
        run_setup_agent(&main, &source, &dir, agent, stderr)?;
    }
    Ok(flake)
}

/// Point the template at `source`, name the VM after the project and
/// authorize `key`.
fn customize_flake(text: &str, source: &str, name: &str, key: Option<&str>) -> String {
    let mut named = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|line| {
            let indent = &line[..line.len() - line.trim_start().len()];
            let trimmed = line.trim_start();
            if trimmed.starts_with("inputs.agent-sandbox.url =") {
                format!("{indent}inputs.agent-sandbox.url = \"{source}\";")
            } else if !named && trimmed.starts_with("name = \"") {
                named = true;
                format!("{indent}name = \"{name}\";")
            } else {
                line.to_string()
            }
        })
        .collect();
    if text.ends_with('\n') {
        out.push(String::new());
    }
    let joined = out.join("\n");
    match key {
        Some(key) => joined.replace(KEY_PLACEHOLDER, key),
        None => joined,
    }
}

/// A VM name from the checkout directory: lower case letters, digits and
/// dashes, as the guest host name `agent-<name>` needs.
fn project_name(main: &Path) -> String {
    let raw = main
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

fn find_public_key() -> Option<String> {
    let ssh = dirs::home_dir()?.join(".ssh");
    ["id_ed25519.pub", "id_ecdsa.pub", "id_rsa.pub"]
        .iter()
        .find_map(|file| read_public_key(&ssh.join(file)).ok())
        .or_else(|| {
            exec::combined_output("ssh-add", &["-L"], None)
                .ok()?
                .lines()
                .find(|line| line.starts_with("ssh-"))
                .map(str::to_string)
        })
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

fn append_line(path: &Path, line: &str) -> Result<(), Error> {
    let mut text = fs::read_to_string(path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    fs::write(path, text).map_err(|err| Error::msg(format!("write {}: {err}", path.display())))
}

#[derive(Deserialize)]
struct Metadata {
    path: PathBuf,
}

/// Start `agent` in the main checkout with agent-sandbox's setup instructions.
fn run_setup_agent(
    main: &Path,
    source: &str,
    dir: &Path,
    agent: &str,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let json = exec::combined_output("nix", &["flake", "metadata", "--json", source], None)?;
    let metadata: Metadata = serde_json::from_str(json.lines().last().unwrap_or_default())
        .map_err(|err| Error::msg(format!("parse nix flake metadata: {err}")))?;
    let instructions = fs::read_to_string(metadata.path.join(SETUP_PROMPT))
        .map_err(|err| Error::msg(format!("read {SETUP_PROMPT} from {source}: {err}")))?;
    let prompt = format!(
        "{instructions}\n\nThe flake to tailor is {}/flake.nix.",
        dir.display()
    );
    exec::require(agent)?;
    writeln!(stderr, "Starting {agent} to tailor the sandbox flake")?;
    exec::run_on_stderr_in(agent, &[&prompt], Some(main), &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = r#"{
  inputs.agent-sandbox.url = "path:../agent-sandbox";
  outputs = { agent-sandbox, ... }:
    agent-sandbox.lib.mkFlake {
      name = "my-nodejs-project";
      modules = [ { agentSandbox.ssh.authorizedKeys = [ "ssh-ed25519 AAAA... your-key-comment" ]; name = "other"; } ];
    };
}
"#;

    #[test]
    fn customizes_the_template() {
        let text = customize_flake(
            TEMPLATE,
            "github:me/agent-sandbox",
            "cli-toolbox",
            Some("ssh-ed25519 AAAAC3 me@mac"),
        );
        assert!(
            text.contains("  inputs.agent-sandbox.url = \"github:me/agent-sandbox\";"),
            "{text}"
        );
        assert!(text.contains("      name = \"cli-toolbox\";"), "{text}");
        assert!(text.contains("\"ssh-ed25519 AAAAC3 me@mac\""), "{text}");
        assert!(
            text.contains("name = \"other\""),
            "only the first name changes"
        );
        assert!(text.ends_with("}\n"));

        let without_key = customize_flake(TEMPLATE, "x", "y", None);
        assert!(without_key.contains(KEY_PLACEHOLDER));
    }

    #[test]
    fn project_names_are_host_name_safe() {
        assert_eq!(project_name(Path::new("/p/My_App.v2")), "my-app-v2");
        assert_eq!(project_name(Path::new("/p/cli-toolbox")), "cli-toolbox");
        assert_eq!(project_name(Path::new("/")), "project");
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

    #[test]
    fn appends_ignore_lines() {
        let dir = tempfile::tempdir().unwrap();
        let ignore = dir.path().join(".gitignore");
        append_line(&ignore, ".agent-sandbox/").unwrap();
        fs::write(&ignore, "/target").unwrap();
        append_line(&ignore, ".agent-sandbox/").unwrap();
        assert_eq!(
            fs::read_to_string(&ignore).unwrap(),
            "/target\n.agent-sandbox/\n"
        );
    }
}
