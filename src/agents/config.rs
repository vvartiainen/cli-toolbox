use super::*;

/// Personal choices of a checkout, in its private directory (`private_dir`).
pub(super) const CHOICES: &str = "choices.json";
pub(super) const GROUPS: [&str; 2] = ["mcp", "skills"];
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
/// The checkout's private directory under `state`: its folder name for
/// reading, and a hash of its path so each checkout and worktree has its own.
pub(super) fn private_dir(state: &Path, root: &Path) -> PathBuf {
    let name: String = root
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let hash = digest(root.as_os_str().as_encoded_bytes());
    state
        .join("projects")
        .join(format!("{name}-{}", &hash[..12]))
}
pub(crate) struct Environment {
    pub catalog: Value,
    pub library: PathBuf,
    /// Personal choices of each checkout.
    pub state: PathBuf,
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
        let state = std::env::var_os("XDG_STATE_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"));
        if !config.is_absolute() || !data.is_absolute() || !state.is_absolute() {
            return Err(Error::msg(
                "XDG config, data and state directories must be absolute",
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
            state: state.join("cli-toolbox/agents"),
        })
    }
    pub fn with_project<T>(
        &self,
        action: impl FnOnce(&mut Project) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let root = project_root(&std::env::current_dir()?)?;
        let _project_lock = lock(&root)?;
        let _library_lock = lock(&self.library)?;
        action(&mut Project::new(
            &root,
            self.catalog.clone(),
            &self.library,
            &self.state,
        )?)
    }
    pub fn install(&self, update: bool) -> Result<(), Error> {
        install_library(&self.library, &self.catalog, &Rulesync::default(), update)
    }
}
