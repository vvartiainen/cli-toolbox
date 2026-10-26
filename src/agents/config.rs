use super::*;

pub(super) const SHARED: &str = ".cli-toolbox/agents.json";
pub(super) const PERSONAL: &str = ".cli-toolbox/agents.local.json";
pub(super) const STATE: &str = ".cli-toolbox/agents.state.json";
pub(super) const GROUPS: [&str; 3] = ["targets", "mcp", "skills"];
pub(super) const TARGETS: [&str; 6] = [
    "codexcli",
    "claudecode",
    "opencode",
    "cursor",
    "copilotcli",
    "pi",
];
pub(super) struct Target {
    pub mcp: &'static [&'static str],
    pub key: &'static str,
    pub skills: &'static str,
}
pub(super) fn target_spec(name: &str) -> Result<Target, Error> {
    let (mcp, key, skills): (&[&str], _, _) = match name {
        "codexcli" => (&[".codex/config.toml"], "mcp_servers", ".agents/skills"),
        "claudecode" => (&[".mcp.json"], "mcpServers", ".claude/skills"),
        "opencode" => (
            &["opencode.jsonc", "opencode.json"],
            "mcp",
            ".opencode/skills",
        ),
        "cursor" => (&[".cursor/mcp.json"], "mcpServers", ".cursor/skills"),
        "copilotcli" => (&[".github/mcp.json"], "mcpServers", ".github/skills"),
        "pi" => (&[".pi/mcp.json"], "mcpServers", ".pi/skills"),
        _ => return Err(Error::msg(format!("unknown target: {name}"))),
    };
    Ok(Target { mcp, key, skills })
}
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.split(['-', '_']).all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}
pub(super) fn object(value: &Value) -> Result<&serde_json::Map<String, Value>, Error> {
    value
        .as_object()
        .ok_or_else(|| Error::msg("expected an object"))
}
pub(super) fn validate_catalog(catalog: &Value) -> Result<(), Error> {
    if catalog.get("version") != Some(&json!(1)) {
        return Err(Error::msg("unsupported catalog version"));
    }
    let targets = catalog["targets"]
        .as_array()
        .ok_or_else(|| Error::msg("catalog targets must name supported Rulesync targets"))?;
    if targets.is_empty()
        || targets
            .iter()
            .any(|v| !v.as_str().is_some_and(|t| TARGETS.contains(&t)))
    {
        return Err(Error::msg(
            "catalog targets must name supported Rulesync targets",
        ));
    }
    let mut aliases = BTreeSet::new();
    for (name, server) in object(&catalog["mcp"])? {
        if !valid_name(name) || !aliases.insert(name.replace('-', "_")) {
            return Err(Error::msg(format!("unsafe or ambiguous MCP name: {name}")));
        }
        if !server.is_object()
            || ["enabled", "disabled", "targets"]
                .iter()
                .any(|k| server.get(k).is_some())
        {
            return Err(Error::msg(format!(
                "use selections, not enabled/disabled/targets, on MCP server {name}"
            )));
        }
    }
    if let Some(local) = catalog.get("localSkills") {
        let local =
            object(local).map_err(|_| Error::msg("catalog localSkills must be an object"))?;
        for (name, path) in local {
            if !valid_name(name) || !path.as_str().is_some_and(|p| Path::new(p).is_absolute()) {
                return Err(Error::msg(format!(
                    "local skill {name} needs a safe name and an absolute path"
                )));
            }
        }
    }
    let sources = catalog["sources"]
        .as_array()
        .ok_or_else(|| Error::msg("catalog requires sources"))?;
    for source in sources {
        if !source.is_object() || !source["source"].is_string() || source.get("rules").is_some() {
            return Err(Error::msg("catalog sources must be skill sources"));
        }
    }
    Ok(())
}

pub(super) fn run_command(executable: &str, args: &[&str], cwd: &Path) -> Result<String, Error> {
    let output = std::process::Command::new(executable)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| Error::msg(format!("run {executable}: {e}")))?;
    if !output.status.success() {
        return Err(Error::msg(format!(
            "{executable} {} failed:\n{}{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        )));
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(Error::msg)
}
pub(crate) fn project_root(cwd: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(run_command("git", &["rev-parse", "--show-toplevel"], cwd)?)
        .map_err(Into::into)
}
const EXCLUDE_BEGIN: &str = "# cli-toolbox agents: generated files";
const EXCLUDE_END: &str = "# end cli-toolbox agents";
/// `generated` replaces the block of excluded generated files; `None` keeps it.
pub(super) fn exclusion_change(
    root: &Path,
    generated: Option<&[String]>,
) -> Result<(PathBuf, Vec<u8>), Error> {
    let path = PathBuf::from(run_command(
        "git",
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "info/exclude",
        ],
        root,
    )?);
    // The location belongs to Git, and may be in the main checkout for a worktree.
    let mut current = Some(path.as_path());
    while let Some(part) = current {
        reject_symlink(part)?;
        current = part.parent();
    }
    let mut old = if path.exists() {
        file_record(&path)?;
        fs::read(&path)?
    } else {
        Vec::new()
    };
    if let Some(generated) = generated {
        let text = String::from_utf8(old).map_err(Error::msg)?;
        let mut kept = String::new();
        let mut inside = false;
        for line in text.lines() {
            match line {
                EXCLUDE_BEGIN => inside = true,
                EXCLUDE_END => inside = false,
                _ if !inside => {
                    kept.push_str(line);
                    kept.push('\n');
                }
                _ => {}
            }
        }
        if !generated.is_empty() {
            kept.push_str(EXCLUDE_BEGIN);
            kept.push('\n');
            for path in generated {
                kept.push_str(&format!("/{path}\n"));
            }
            kept.push_str(EXCLUDE_END);
            kept.push('\n');
        }
        old = kept.into_bytes();
    }
    let text = std::str::from_utf8(&old).map_err(Error::msg)?;
    let additions: Vec<_> = [PERSONAL, STATE]
        .iter()
        .map(|p| format!("/{p}"))
        .filter(|p| !text.lines().any(|line| line == p))
        .collect();
    if !additions.is_empty() {
        if !old.is_empty() && !old.ends_with(b"\n") {
            old.push(b'\n');
        }
        old.extend_from_slice(format!("{}\n", additions.join("\n")).as_bytes());
    }
    Ok((path, old))
}

pub(crate) struct Environment {
    pub catalog: Value,
    pub library: PathBuf,
}
impl Environment {
    pub fn discover() -> Result<Self, Error> {
        let home = dirs::home_dir().ok_or_else(|| Error::msg("home directory not found"))?;
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        if !config.is_absolute() || !data.is_absolute() {
            return Err(Error::msg(
                "XDG config and data directories must be absolute",
            ));
        }
        let path = config.join("cli-toolbox/agents/catalog.json");
        let catalog = load_json(&path, Value::Null)?;
        if catalog.is_null() {
            return Err(Error::msg(format!(
                "agent catalog is missing: {}",
                path.display()
            )));
        }
        validate_catalog(&catalog)?;
        Ok(Self {
            catalog,
            library: data.join("cli-toolbox/agents/library"),
        })
    }
    pub fn with_project<T>(
        &self,
        action: impl FnOnce(&mut Project<'_>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let root = project_root(&std::env::current_dir()?)?;
        let _project_lock = lock(&root)?;
        let _library_lock = lock(&self.library)?;
        let backend = Rulesync::default();
        action(&mut Project::new(
            &root,
            self.catalog.clone(),
            &self.library,
            &backend,
        )?)
    }
    pub fn install(&self, update: bool) -> Result<(), Error> {
        install_library(&self.library, &self.catalog, &Rulesync::default(), update)
    }
}
