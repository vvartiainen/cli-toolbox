use super::*;
use serde::Serialize;

/// The plugin that carries chosen MCP servers and skills into Claude Code and
/// skills into Copilot.
const PLUGIN: &str = "ct-agents";

/// A coding agent `ct agents launch` starts with the chosen tools.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Agent {
    Claude,
    Codex,
    Opencode,
    Copilot,
    Pi,
}
impl Agent {
    pub const ALL: [Self; 5] = [
        Self::Claude,
        Self::Codex,
        Self::Opencode,
        Self::Copilot,
        Self::Pi,
    ];
    /// Its command name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
            Self::Copilot => "copilot",
            Self::Pi => "pi",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.name() == name)
    }
    /// Whether a session starts with these arguments; Pi's management
    /// subcommands reject session options.
    pub fn starts_session(self, args: &[String]) -> bool {
        const PI_COMMANDS: [&str; 8] = [
            "install",
            "remove",
            "uninstall",
            "update",
            "list",
            "config",
            "auth",
            "mcp",
        ];
        self != Self::Pi
            || !args
                .first()
                .is_some_and(|a| PI_COMMANDS.contains(&a.as_str()))
    }
}

/// The options that give an agent the chosen tools, and what it cannot load.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct Launch {
    pub args: Vec<String>,
    pub notes: Vec<String>,
}

impl Project {
    pub fn launch(&self, agent: Agent) -> Result<Launch, Error> {
        let servers = self.servers();
        let skills = self.chosen_skills();
        let mut launch = Launch::default();
        // What the agent cannot load, by name.
        let skipped = |what: &str, names: Vec<&String>| {
            (!names.is_empty()).then(|| {
                let names: Vec<_> = names.into_iter().map(String::as_str).collect();
                format!(
                    "{} cannot load {what} at launch; skipped {}",
                    agent.name(),
                    names.join(", ")
                )
            })
        };
        match agent {
            Agent::Claude => {
                if !servers.is_empty() || !skills.is_empty() {
                    let plugin = self.plugin(&servers, &skills)?;
                    launch.args = vec!["--plugin-dir".into(), utf8(&plugin)?.into()];
                }
            }
            Agent::Copilot => {
                if !skills.is_empty() {
                    let plugin = self.plugin(&serde_json::Map::new(), &skills)?;
                    launch.args = vec!["--plugin-dir".into(), utf8(&plugin)?.into()];
                }
                if !servers.is_empty() {
                    let servers: serde_json::Map<_, _> = servers
                        .iter()
                        .map(|(name, server)| {
                            let mut server = stdio(server);
                            server["type"] = json!("local");
                            server["tools"] = json!(["*"]);
                            (name.clone(), server)
                        })
                        .collect();
                    launch.args.extend([
                        "--additional-mcp-config".into(),
                        json!({ "mcpServers": servers }).to_string(),
                    ]);
                }
            }
            Agent::Codex => {
                for (name, server) in &servers {
                    let value = stdio(server)
                        .serialize(toml_edit::ser::ValueSerializer::new())
                        .map_err(Error::msg)?;
                    launch
                        .args
                        .extend(["-c".into(), format!("mcp_servers.{name}={value}")]);
                }
                launch
                    .notes
                    .extend(skipped("skills", skills.keys().collect()));
            }
            // OpenCode sessions share a background server that keeps the
            // configuration it started with, so per-launch tools would leak
            // between repositories.
            Agent::Opencode => {
                launch
                    .notes
                    .extend(skipped("MCP servers", servers.keys().collect()));
                launch
                    .notes
                    .extend(skipped("skills", skills.keys().collect()));
            }
            Agent::Pi => {
                for path in skills.values() {
                    launch.args.extend(["--skill".into(), utf8(path)?.into()]);
                }
                launch
                    .notes
                    .extend(skipped("MCP servers", servers.keys().collect()));
            }
        }
        Ok(launch)
    }
    /// A Claude Code plugin with the servers and skills, shared by every launch
    /// that chooses the same: it is named after a hash of its content.
    fn plugin(
        &self,
        servers: &serde_json::Map<String, Value>,
        skills: &BTreeMap<String, PathBuf>,
    ) -> Result<PathBuf, Error> {
        let mut records = BTreeMap::new();
        for (name, path) in skills {
            records.insert(name, tree_records(path)?);
        }
        let hash = digest(&json_bytes(&json!({"servers":servers,"skills":records}))?);
        let launches = self.private.join("launch");
        let plugin = launches.join(&hash[..16]);
        if plugin.is_dir() {
            return Ok(plugin);
        }
        fs::create_dir_all(&launches)?;
        let staging = tempfile::tempdir_in(&launches)?.keep();
        write_json(
            &staging.join(".claude-plugin/plugin.json"),
            &json!({"name":PLUGIN,"description":"MCP servers and skills chosen with ct agents"}),
        )?;
        if !servers.is_empty() {
            let servers: serde_json::Map<_, _> = servers
                .iter()
                .map(|(name, server)| (name.clone(), stdio(server)))
                .collect();
            write_json(
                &staging.join(".mcp.json"),
                &json!({ "mcpServers": servers }),
            )?;
        }
        for (name, path) in skills {
            copy_tree(path, &staging.join("skills").join(name))?;
        }
        // Another launch may have made the same plugin meanwhile.
        if fs::rename(&staging, &plugin).is_err() {
            fs::remove_dir_all(&staging)?;
        }
        Ok(plugin)
    }
}
/// A catalog server as a stdio server entry: command, args and env.
fn stdio(server: &Value) -> Value {
    let mut server = server.clone();
    if let Some(server) = server.as_object_mut() {
        server.remove("type");
    }
    server
}
fn utf8(path: &Path) -> Result<&str, Error> {
    path.to_str()
        .ok_or_else(|| Error::msg(format!("non-UTF-8 path: {}", path.display())))
}
