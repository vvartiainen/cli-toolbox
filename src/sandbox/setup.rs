//! The shared setup every sandbox project gets: agent credentials from
//! agent-proxy on the host, and a Backlog.md backlog served to the guest.
//! `project create` writes them into the flake; the host side (the proxy
//! client, its token and CA, and the backlog folder) is set up here, from
//! what the flake says, so it also works for existing projects.

use std::ffi::OsStr;
use std::path::PathBuf;

use super::project::{self, Project};
use crate::{Error, exec};

/// The folder that holds one backlog per project, `<root>/<project>`.
/// Empty disables backlogs for new projects.
pub(crate) const BACKLOG_ROOT_ENV: &str = "CLI_TOOLBOX_BACKLOG_ROOT";
/// Used when it exists and `$CLI_TOOLBOX_BACKLOG_ROOT` is unset.
const DEFAULT_BACKLOG_ROOT: &str = "Library/Mobile Documents/iCloud~md~obsidian/Documents/Agents";

/// The agent-proxy services the defaults module's credentials come from.
const PROXY_SERVICES: &str = "claude-code,codex";
/// In the agent-sandbox repository.
const PROXY_SETUP_SCRIPT: &str = "scripts/agent-proxy-setup.sh";
pub(crate) const PROXY_CA_FILE: &str = "agent-proxy-ca.pem";

/// The backlog root for new projects, if any.
pub(crate) fn backlog_root() -> Option<PathBuf> {
    match std::env::var_os(BACKLOG_ROOT_ENV) {
        Some(root) if root.is_empty() => None,
        Some(root) => Some(PathBuf::from(root)),
        None => dirs::home_dir()
            .map(|home| home.join(DEFAULT_BACKLOG_ROOT))
            .filter(|root| root.is_dir()),
    }
}

/// The environment module `project create` adds: proxy credentials when
/// `proxy`, the shared agent tools, and the backlog when it has a folder.
pub(crate) fn defaults_module(name: &str, proxy: bool, backlog: bool, indent: &str) -> String {
    let mut lines = vec![
        "# cli-toolbox defaults: agent credentials from agent-proxy on the host and".into(),
        "# the shared agent tools (`cli-toolbox agent project setup` sets up the host).".into(),
        "{".into(),
    ];
    if proxy {
        lines.extend(
            [
                "  proxy = {",
                "    enable = true;",
                &format!("    client = {};", project::nix_string(name)),
                &format!("    caCert = ./{PROXY_CA_FILE};"),
                "    env = {",
                "      CLAUDE_CODE_OAUTH_TOKEN = \"sk-ant-oat01-proxy-managed\";",
                "      ANTHROPIC_OAUTH_TOKEN = \"sk-ant-oat01-proxy-managed\";",
                "    };",
                "  };",
            ]
            .map(String::from),
        );
    }
    lines.push("  ai = {".into());
    if proxy {
        lines.push("    chatgptLoginFromProxy = true;".into());
    }
    lines.extend(["    claude.enable = true;", "    codex.enable = true;"].map(String::from));
    if backlog {
        lines.push("    backlog.enable = true;".into());
    }
    lines.extend(["  };", "}"].map(String::from));
    lines
        .iter()
        .map(|line| format!("{indent}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether the flake takes its credentials from agent-proxy.
pub(crate) fn uses_proxy(flake: &str) -> bool {
    flake
        .lines()
        .any(|line| line.trim() == format!("caCert = ./{PROXY_CA_FILE};"))
}

/// `ai` options that enable something other than a coding agent.
const NOT_AGENTS: [&str; 1] = ["backlog"];

/// The coding agents the flake enables, such as `ai.claude.enable = true;`,
/// in order. Their names are the agent kinds Herdr starts. Reads both the
/// nested form the defaults module writes and dotted attribute paths.
pub(crate) fn enabled_agents(flake: &str) -> Vec<String> {
    let mut agents = Vec::new();
    // The attribute names of the open `name = {` blocks; empty for others.
    let mut open: Vec<String> = Vec::new();
    for line in flake.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('}') {
            open.pop();
            continue;
        }
        if let Some(head) = line.strip_suffix('{') {
            open.push(head.trim().trim_end_matches('=').trim().into());
            continue;
        }
        let Some((name, value)) = line.strip_suffix(';').and_then(|line| line.split_once('='))
        else {
            continue;
        };
        if value.trim() != "true" {
            continue;
        }
        let path: Vec<&str> = open
            .iter()
            .flat_map(|name| name.split('.'))
            .chain(name.trim().split('.'))
            .collect();
        if let [.., "ai", agent, "enable"] = path.as_slice()
            && !NOT_AGENTS.contains(agent)
            && !agents.iter().any(|known| known == agent)
        {
            agents.push(agent.to_string());
        }
    }
    agents
}

/// The flake's `vm.backlogDir`.
pub(crate) fn backlog_dir(flake: &str) -> Option<PathBuf> {
    string_setting(flake, "backlogDir =").map(PathBuf::from)
}

/// The agent-sandbox flake the project's flake uses.
fn agent_sandbox_input(flake: &str) -> Option<String> {
    string_setting(flake, "inputs.agent-sandbox.url =")
}

/// The string value of a `key = "value";` line.
fn string_setting(flake: &str, key: &str) -> Option<String> {
    flake.lines().find_map(|line| {
        let value = line.trim().strip_prefix(key)?.trim();
        let value = value.strip_suffix(';')?.trim();
        let value = value.strip_prefix('"')?.strip_suffix('"')?;
        Some(value.replace("\\\"", "\"").replace("\\\\", "\\"))
    })
}

/// Run agent-sandbox's proxy setup for the project: store missing service
/// secrets (prompting for them), create the client and its keychain token,
/// and export the CA next to the flake. Safe to re-run. `false` when the
/// flake does not use the proxy.
pub(crate) fn proxy(project: &Project, flake: &str) -> Result<bool, Error> {
    if !uses_proxy(flake) {
        return Ok(false);
    }
    let source = project::source(agent_sandbox_input(flake).as_deref());
    let script = project::source_dir(&source)?.join(PROXY_SETUP_SCRIPT);
    let script = script.to_string_lossy();
    let dir = project.dir.to_string_lossy();
    exec::require("bash")?;
    exec::run_on_stderr_in(
        "bash",
        &[
            &script,
            "--dir",
            &dir,
            "--client",
            &project.name,
            "--services",
            PROXY_SERVICES,
        ],
        Some(&project.dir),
        &[],
    )?;
    Ok(true)
}

/// What [`backlog`] did.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Backlog {
    /// The flake has no `backlogDir`.
    Disabled,
    Initialized(PathBuf),
    Exists(PathBuf),
}

/// Create the flake's backlog folder and initialize Backlog.md in it, unless
/// it already has a backlog.
pub(crate) fn backlog(project: &Project, flake: &str) -> Result<Backlog, Error> {
    let Some(dir) = backlog_dir(flake) else {
        return Ok(Backlog::Disabled);
    };
    if dir.join("backlog/config.yml").is_file() {
        return Ok(Backlog::Exists(dir));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|err| Error::msg(format!("create {}: {err}", dir.display())))?;
    exec::require("backlog")?;
    exec::run_on_stderr_in(
        "backlog",
        &[
            "init",
            &project.name,
            "--defaults",
            "--no-git",
            "--integration-mode",
            "none",
        ],
        Some(&dir),
        &[("BACKLOG_CWD", OsStr::new(&dir))],
    )?;
    Ok(Backlog::Initialized(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_module_takes_credentials_from_the_proxy() {
        let module = defaults_module("demo", true, true, "  ");
        for want in [
            "  {\n",
            "    proxy = {\n      enable = true;\n      client = \"demo\";\n      caCert = ./agent-proxy-ca.pem;\n",
            "CLAUDE_CODE_OAUTH_TOKEN = \"sk-ant-oat01-proxy-managed\";",
            "      chatgptLoginFromProxy = true;\n      claude.enable = true;\n      codex.enable = true;\n      backlog.enable = true;\n    };\n  }",
        ] {
            assert!(module.contains(want), "missing {want}:\n{module}");
        }
        assert!(uses_proxy(&module));

        let plain = defaults_module("demo", false, false, "");
        assert!(!plain.contains("proxy = {"), "{plain}");
        assert!(!plain.contains("chatgptLoginFromProxy"), "{plain}");
        assert!(!plain.contains("backlog.enable"), "{plain}");
        assert!(plain.contains("claude.enable = true;"), "{plain}");
    }

    #[test]
    fn enabled_agents_come_from_the_ai_options() {
        let module = defaults_module("demo", true, true, "        ");
        let flake = format!(
            "{{\n  outputs = {{ self, agent-sandbox }}: {{\n    modules = [\n{module}\n    ];\n  }};\n}}\n"
        );
        assert_eq!(enabled_agents(&flake), ["claude", "codex"]);
        let flake = "ai = {\n  claude.enable = false;\n  codex = {\n    enable = true;\n  };\n};\nai.gemini.enable = true; # on\n# ai.pi.enable = true;\nvm.enable = true;\n";
        assert_eq!(enabled_agents(flake), ["codex", "gemini"]);
        assert!(enabled_agents("{ }").is_empty());
    }

    #[test]
    fn reads_settings_from_the_flake() {
        let flake = r#"
  inputs.agent-sandbox.url = "path:/p/agent-sandbox";
          caCert = ./agent-proxy-ca.pem;
        backlogDir = "/Users/me/Agents/demo";
"#;
        assert!(uses_proxy(flake));
        assert_eq!(
            backlog_dir(flake),
            Some(PathBuf::from("/Users/me/Agents/demo"))
        );
        assert_eq!(
            agent_sandbox_input(flake).as_deref(),
            Some("path:/p/agent-sandbox")
        );
        assert!(!uses_proxy("# caCert = ./agent-proxy-ca.pem;"));
        assert_eq!(backlog_dir("{ }"), None);
    }

    #[test]
    fn an_existing_backlog_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("backlog")).unwrap();
        std::fs::write(
            dir.path().join("backlog/config.yml"),
            "project_name: demo\n",
        )
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("projects/demo");
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join("flake.nix"), "{ }").unwrap();
        let project = project::load(root.path(), "demo").unwrap();
        let flake = format!(
            "backlogDir = {};",
            project::nix_string(&dir.path().to_string_lossy())
        );
        assert_eq!(
            backlog(&project, &flake).unwrap(),
            Backlog::Exists(dir.path().into())
        );
        assert_eq!(backlog(&project, "{ }").unwrap(), Backlog::Disabled);
    }
}
