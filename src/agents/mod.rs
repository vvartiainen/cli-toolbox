use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::Error;
use serde_json::{Value, json};

mod backend;
mod config;
mod files;
mod library;
mod native;
use backend::*;
use config::*;
use files::*;
use library::*;
use native::*;

#[cfg(test)]
mod tests;

pub(crate) use config::Environment;

/// The choices of the working directory's repository, for the TUI.
pub(crate) fn overview() -> Result<Overview, Error> {
    Environment::discover()?.with_project(|project| project.overview())
}
/// Save one choice of the working directory's repository without syncing.
pub(crate) fn set_choice(group: &str, name: &str, choice: &str, shared: bool) -> Result<(), Error> {
    Environment::discover()?.with_project(|project| project.set_choice(group, name, choice, shared))
}
/// Keep the working directory's generated files out of Git, or stop doing so.
pub(crate) fn set_exclude(exclude: bool) -> Result<(), Error> {
    Environment::discover()?.with_project(|project| project.set_exclude(exclude))
}
/// Sync, or preview with `dry_run`, the working directory's repository.
pub(crate) fn sync(dry_run: bool, out: &mut dyn Write) -> Result<bool, Error> {
    Environment::discover()?.with_project(|project| project.sync(dry_run, &[], out))
}
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetState {
    servers: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
    targets: BTreeMap<String, TargetState>,
    skills: Records,
    files: Records,
}
impl Default for State {
    fn default() -> Self {
        Self {
            version: 1,
            signature: None,
            targets: BTreeMap::new(),
            skills: Records::new(),
            files: Records::new(),
        }
    }
}
/// One selectable MCP server, skill or target with its value in each layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub group: String,
    pub name: String,
    /// The catalog default.
    pub default: bool,
    /// The shared (repository) choice; none when it inherits.
    pub shared: Option<bool>,
    /// The personal choice; none when it inherits.
    pub personal: Option<bool>,
}
impl Entry {
    pub fn enabled(&self) -> bool {
        self.personal.or(self.shared).unwrap_or(self.default)
    }
    /// `personal`, `shared` or `default`: the layer the value comes from.
    pub fn origin(&self) -> &'static str {
        if self.personal.is_some() {
            "personal"
        } else if self.shared.is_some() {
            "shared"
        } else {
            "default"
        }
    }
    /// The next choice when cycling a layer: from inheriting to the opposite
    /// of the inherited value, then the inherited value, then inheriting again.
    pub fn next_choice(&self, shared: bool) -> &'static str {
        let (current, inherited) = if shared {
            (self.shared, self.default)
        } else {
            (self.personal, self.shared.unwrap_or(self.default))
        };
        let choice = |on| if on { "enable" } else { "disable" };
        match current {
            None => choice(!inherited),
            Some(on) if on != inherited => choice(inherited),
            Some(_) => "inherit",
        }
    }
}
/// A project's effective choices, for status output and the TUI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Overview {
    pub entries: Vec<Entry>,
    pub exclude: bool,
    pub synced: bool,
}
pub(crate) struct Project<'a> {
    root: PathBuf,
    catalog: Value,
    library: PathBuf,
    backend: &'a dyn Backend,
    skills: BTreeMap<String, PathBuf>,
    names: BTreeMap<String, Vec<String>>,
    shared: Value,
    personal: Value,
    effective: Value,
    state: State,
    inputs: BTreeMap<PathBuf, Option<Record>>,
}
impl<'a> Project<'a> {
    fn new(
        root: &Path,
        catalog: Value,
        library: &Path,
        backend: &'a dyn Backend,
    ) -> Result<Self, Error> {
        validate_catalog(&catalog)?;
        let root = root.canonicalize()?;
        let skills = library_skills(library, &catalog)?;
        let names = BTreeMap::from([
            (
                "targets".into(),
                catalog["targets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_str().unwrap().to_owned())
                    .collect(),
            ),
            (
                "mcp".into(),
                object(&catalog["mcp"])?.keys().cloned().collect(),
            ),
            ("skills".into(), skills.keys().cloned().collect()),
        ]);
        let mut inputs = BTreeMap::new();
        for relative in [SHARED, PERSONAL, STATE] {
            let path = safe_path(&root, relative)?;
            inputs.insert(path.clone(), optional_record(&path)?);
        }
        let shared = load_json(&safe_path(&root, SHARED)?, json!({"version":1}))?;
        let personal = load_json(&safe_path(&root, PERSONAL)?, json!({"version":1}))?;
        for layer in [&shared, &personal] {
            if layer.get("version") != Some(&json!(1)) {
                return Err(Error::msg("unsupported choices version"));
            }
        }
        let value = load_json(
            &safe_path(&root, STATE)?,
            serde_json::to_value(State::default()).map_err(Error::msg)?,
        )?;
        if value.get("version") != Some(&json!(1)) {
            return Err(Error::msg(
                "unsupported state version; restore the original ownership manifest",
            ));
        }
        let state: State =
            serde_json::from_value(value).map_err(|e| Error::msg(format!("invalid state: {e}")))?;
        for (target, previous) in &state.targets {
            target_spec(target)?;
            if previous.servers.keys().any(|n| !valid_name(n)) {
                return Err(Error::msg("invalid owned MCP name"));
            }
        }
        let skill_path = |relative: &str| -> bool {
            TARGETS.iter().any(|t| {
                let prefix = format!("{}/", target_spec(t).unwrap().skills);
                relative.strip_prefix(&prefix).is_some_and(|rest| {
                    rest.split_once('/')
                        .is_some_and(|(name, file)| valid_name(name) && !file.is_empty())
                })
            })
        };
        for (relative, record) in &state.skills {
            safe_path(&root, relative)?;
            if !skill_path(relative) || state.files.get(relative) != Some(record) {
                return Err(Error::msg(format!("invalid owned skill path: {relative}")));
            }
        }
        for (relative, record) in &state.files {
            safe_path(&root, relative)?;
            let is_mcp = TARGETS
                .iter()
                .any(|t| target_spec(t).unwrap().mcp.contains(&relative.as_str()));
            if (!state.skills.contains_key(relative) && !is_mcp)
                || record.sha256.len() != 64
                || !record.sha256.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(Error::msg(format!(
                    "invalid owned file path or hash: {relative}"
                )));
            }
        }
        let mut project = Self {
            root,
            catalog,
            library: library.to_path_buf(),
            backend,
            skills,
            names,
            shared,
            personal,
            effective: json!({}),
            state,
            inputs,
        };
        for layer in [
            &project.shared,
            &project.personal,
            &project
                .catalog
                .get("defaults")
                .cloned()
                .unwrap_or_else(|| json!({})),
        ] {
            project.validate_choices(layer)?;
        }
        if project.shared.get("exclude").is_some()
            || project.catalog["defaults"].get("exclude").is_some()
        {
            return Err(Error::msg("exclude is a personal choice"));
        }
        project.resolve_choices();
        Ok(project)
    }
    fn validate_choices(&self, choices: &Value) -> Result<(), Error> {
        let choices = object(choices)?;
        for group in choices.keys() {
            if !GROUPS.contains(&group.as_str()) && group != "version" && group != "exclude" {
                return Err(Error::msg(format!("unknown selection group: {group}")));
            }
        }
        if choices.get("exclude").is_some_and(|v| !v.is_boolean()) {
            return Err(Error::msg("exclude must be true or false"));
        }
        for group in GROUPS {
            if let Some(values) = choices.get(group) {
                for (name, choice) in object(values)? {
                    if !self.names[group].contains(name) {
                        return Err(Error::msg(format!("unknown {group} selection: {name}")));
                    }
                    if !choice
                        .as_str()
                        .is_some_and(|s| ["inherit", "enable", "disable"].contains(&s))
                    {
                        return Err(Error::msg(format!("unknown selection choice: {choice}")));
                    }
                }
            }
        }
        Ok(())
    }
    fn resolve_choices(&mut self) {
        self.effective = json!({});
        for group in GROUPS {
            self.effective[group] = json!({});
            for name in &self.names[group] {
                self.effective[group][name] = json!(false);
            }
        }
        for layer in [&self.catalog["defaults"], &self.shared, &self.personal] {
            for group in GROUPS {
                if let Some(values) = layer[group].as_object() {
                    for (name, choice) in values {
                        if choice != "inherit" {
                            self.effective[group][name] = json!(choice == "enable");
                        }
                    }
                }
            }
        }
    }
    fn enabled(&self, group: &str, name: &str) -> bool {
        self.effective[group][name] == true
    }
    fn excluded(&self) -> bool {
        self.personal["exclude"] == true
    }
    /// Keep generated files out of Git through its local exclude file.
    pub fn set_exclude(&mut self, exclude: bool) -> Result<(), Error> {
        let mut layer = self.personal.clone();
        if exclude {
            layer["exclude"] = json!(true);
        } else if let Some(layer) = layer.as_object_mut() {
            layer.remove("exclude");
        }
        self.save_choices(layer, false)
    }
    fn signature(&self) -> Result<String, Error> {
        let mut local = BTreeMap::new();
        if let Some(skills) = self.catalog.get("localSkills") {
            for (name, path) in object(skills)? {
                local.insert(name, tree_records(Path::new(path.as_str().unwrap()))?);
            }
        }
        Ok(digest(&json_bytes(
            &json!({"catalog":self.catalog,"effective":self.effective,"library":tree_records(&safe_path(&self.library, ".rulesync/skills/.curated")?)?,"local":local}),
        )?))
    }
    fn is_synced(&self) -> Result<bool, Error> {
        if self.state.signature.as_deref() != Some(&self.signature()?) {
            return Ok(false);
        }
        for (relative, record) in &self.state.files {
            if optional_record(&safe_path(&self.root, relative)?)?.as_ref() != Some(record) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn verify_inputs(&self) -> Result<(), Error> {
        verify_observed(&self.inputs)
    }
    pub fn set_choice(
        &mut self,
        group: &str,
        name: &str,
        choice: &str,
        shared: bool,
    ) -> Result<(), Error> {
        self.validate_choices(&json!({group:{name:choice}}))?;
        let mut layer = if shared {
            self.shared.clone()
        } else {
            self.personal.clone()
        };
        if layer.get(group).is_none() {
            layer[group] = json!({});
        }
        if choice == "inherit" {
            layer[group].as_object_mut().unwrap().remove(name);
        } else {
            layer[group][name] = json!(choice);
        }
        self.save_choices(layer, shared)
    }
    fn save_choices(&mut self, layer: Value, shared: bool) -> Result<(), Error> {
        self.validate_choices(&layer)?;
        let path = safe_path(&self.root, if shared { SHARED } else { PERSONAL })?;
        let (exclude, _) = exclusion_change(&self.root, None)?;
        let _guard = lock(&exclude)?;
        self.verify_inputs()?;
        let (exclude, content) = exclusion_change(&self.root, None)?;
        apply_changes(&BTreeMap::from([
            (exclude, (Some(content), 0o644)),
            (
                path.clone(),
                (
                    Some(json_bytes(&layer)?),
                    if shared { 0o644 } else { 0o600 },
                ),
            ),
        ]))?;
        self.inputs.insert(path.clone(), optional_record(&path)?);
        if shared {
            self.shared = layer;
        } else {
            self.personal = layer;
        }
        self.resolve_choices();
        Ok(())
    }
    pub fn overview(&self) -> Result<Overview, Error> {
        let mut entries = Vec::new();
        for group in GROUPS {
            for name in &self.names[group] {
                let choice = |layer: &Value| match layer[group].get(name)?.as_str()? {
                    "enable" => Some(true),
                    "disable" => Some(false),
                    _ => None,
                };
                entries.push(Entry {
                    group: group.into(),
                    name: name.clone(),
                    default: choice(&self.catalog["defaults"]).unwrap_or(false),
                    shared: choice(&self.shared),
                    personal: choice(&self.personal),
                });
            }
        }
        Ok(Overview {
            entries,
            exclude: self.excluded(),
            synced: self.is_synced()?,
        })
    }
    pub fn status(&self, out: &mut dyn Write) -> Result<(), Error> {
        let overview = self.overview()?;
        for entry in &overview.entries {
            writeln!(
                out,
                "{} / {}: {} ({})",
                entry.group,
                entry.name,
                if entry.enabled() { "enable" } else { "disable" },
                entry.origin()
            )?;
        }
        writeln!(
            out,
            "exclude generated files: {}",
            if overview.exclude {
                "enable"
            } else {
                "disable"
            }
        )?;
        writeln!(
            out,
            "{}",
            if overview.synced {
                "Synchronized."
            } else {
                "Pending changes or file drift; preview with ct agents sync --dry-run."
            }
        )?;
        Ok(())
    }
    pub fn interactive(&mut self, shared: bool, out: &mut dyn Write) -> Result<(), Error> {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            return Err(Error::msg(
                "interactive selection requires a terminal; use ct agents set",
            ));
        }
        let mut layer = if shared {
            self.shared.clone()
        } else {
            self.personal.clone()
        };
        let entries: Vec<_> = GROUPS
            .into_iter()
            .flat_map(|g| self.names[g].iter().map(move |n| (g, n.clone())))
            .collect();
        loop {
            let mut items: Vec<_> = entries
                .iter()
                .map(|(g, n)| {
                    let selected = layer[*g][n].as_str().unwrap_or("inherit");
                    let mut inherited = "disable";
                    for parent in [&self.catalog["defaults"], &self.shared]
                        .into_iter()
                        .take(if shared { 1 } else { 2 })
                    {
                        if let Some(v) = parent[*g][n].as_str().filter(|v| *v != "inherit") {
                            inherited = v;
                        }
                    }
                    format!("{g} / {n}: {selected} (inherited: {inherited})")
                })
                .collect();
            items.push("Save choices".into());
            let prompt = format!("{} choices > ", if shared { "shared" } else { "personal" });
            let Some(index) = crate::select::pick(&prompt, &items, "~100%")? else {
                return Ok(());
            };
            if index == entries.len() {
                self.save_choices(layer, shared)?;
                writeln!(out, "Choices saved. Preview with ct agents sync --dry-run.")?;
                return Ok(());
            }
            let choices = ["inherit", "enable", "disable"].map(str::to_owned);
            if let Some(index_choice) = crate::select::pick("choice > ", &choices, "40%")? {
                let (group, name) = &entries[index];
                if layer.get(*group).is_none() {
                    layer[*group] = json!({});
                }
                layer[*group][name] = json!(choices[index_choice]);
            }
        }
    }

    pub fn sync(
        &mut self,
        dry_run: bool,
        adopt: &[String],
        out: &mut dyn Write,
    ) -> Result<bool, Error> {
        if adopt.iter().any(|t| !self.names["targets"].contains(t)) {
            return Err(Error::msg("--adopt-mcp must name a configured target"));
        }
        self.verify_inputs()?;
        let signature = self.signature()?;
        let active: BTreeSet<_> = self.names["targets"]
            .iter()
            .filter(|n| self.enabled("targets", n))
            .cloned()
            .collect();
        let selected: serde_json::Map<_, _> = object(&self.catalog["mcp"])?
            .iter()
            .filter(|(n, _)| self.enabled("mcp", n))
            .map(|(n, v)| (n.clone(), v.clone()))
            .collect();
        let mut desired_skills: BTreeMap<String, (Vec<u8>, u32)> = BTreeMap::new();
        let mut generated = BTreeMap::new();
        let temporary = tempfile::Builder::new().prefix("ct-agents-").tempdir()?;
        for target in &active {
            let base = temporary.path().join(target);
            let source = base.join("input");
            let output = base.join("output");
            fs::create_dir_all(source.join("skills"))?;
            fs::create_dir(&output)?;
            write_json(&source.join("mcp.jsonc"), &json!({"mcpServers":selected}))?;
            for (name, path) in &self.skills {
                if self.enabled("skills", name) {
                    copy_tree(path, &source.join("skills").join(name))?;
                }
            }
            self.backend.generate(target, &source, &output)?;
            let spec = target_spec(target)?;
            let outputs: Vec<_> = spec
                .mcp
                .iter()
                .filter(|p| output.join(p).exists())
                .collect();
            if outputs.len() > 1 {
                return Err(Error::msg(format!(
                    "multiple Rulesync MCP outputs for {target}"
                )));
            }
            let servers = if let Some(relative) = outputs.first() {
                Native::read(&safe_path(&output, relative)?)?.servers(spec.key)?
            } else {
                json!({})
            };
            if object(&servers)?.keys().ne(selected.keys()) {
                return Err(Error::msg(format!(
                    "Rulesync omitted or renamed an MCP server for {target}"
                )));
            }
            generated.insert(target.clone(), servers);
            for (relative, rec) in tree_records(&output)? {
                if spec.mcp.contains(&relative.as_str()) {
                    continue;
                }
                if !relative.starts_with(&format!("{}/", spec.skills)) {
                    return Err(Error::msg(format!(
                        "unexpected Rulesync output for {target}: {relative}"
                    )));
                }
                let value = (
                    fs::read(safe_path(&output, &relative)?)?,
                    if rec.executable { 0o755 } else { 0o644 },
                );
                if let Some(previous) = desired_skills.insert(relative.clone(), value.clone())
                    && previous != value
                {
                    return Err(Error::msg(format!(
                        "targets generated different content for {relative}"
                    )));
                }
            }
            for name in self.skills.keys().filter(|n| self.enabled("skills", n)) {
                if !desired_skills.contains_key(&format!("{}/{name}/SKILL.md", spec.skills)) {
                    return Err(Error::msg(format!(
                        "Rulesync omitted skill {name} for {target}"
                    )));
                }
            }
        }
        let mut changes = Changes::new();
        let mut observed = self.inputs.clone();
        let mut next_targets = BTreeMap::new();
        let mut file_states = Records::new();
        let targets: BTreeSet<_> = active
            .iter()
            .chain(self.state.targets.keys())
            .cloned()
            .collect();
        for target in &targets {
            let spec = target_spec(target)?;
            let mut candidates = Vec::new();
            for relative in spec.mcp {
                let path = safe_path(&self.root, relative)?;
                let rec = optional_record(&path)?;
                if rec.is_some() {
                    candidates.push(*relative);
                }
                observed.insert(path, rec);
            }
            if candidates.len() > 1 {
                return Err(Error::msg(format!(
                    "multiple {target} MCP config files; consolidate them before syncing"
                )));
            }
            let relative = candidates
                .first()
                .copied()
                .unwrap_or(spec.mcp.last().unwrap());
            let path = safe_path(&self.root, relative)?;
            let mut document = Native::read(&path)?;
            let current_value = document.servers(spec.key)?;
            let current = object(&current_value)?;
            let previous = self.state.targets.get(target);
            let owned = previous.map(|s| s.servers.clone()).unwrap_or_default();
            let wanted_value = generated.get(target).cloned().unwrap_or_else(|| json!({}));
            let wanted = object(&wanted_value)?;
            if !current.is_empty() && previous.is_none() && !adopt.contains(target) {
                return Err(Error::msg(format!(
                    "existing MCP section in {}; preview with --adopt-mcp {target}",
                    path.display()
                )));
            }
            for (name, entry) in &owned {
                if current.get(name).is_some_and(|v| v != entry) {
                    return Err(Error::msg(format!(
                        "locally edited MCP server {target}/{name}; restore it before syncing"
                    )));
                }
            }
            for name in wanted.keys().filter(|n| !owned.contains_key(*n)) {
                if current.contains_key(name) && !adopt.contains(target) {
                    return Err(Error::msg(format!(
                        "existing MCP server {target}/{name}; use --adopt-mcp {target}"
                    )));
                }
            }
            let mut combined = current.clone();
            for name in owned.keys() {
                combined.remove(name);
            }
            combined.extend(wanted.clone());
            let mode = if path.exists() {
                fs::metadata(&path)?.permissions().mode() & 0o777
            } else {
                0o600
            };
            if &combined != current {
                document.set_servers(spec.key, Value::Object(combined))?;
                changes.insert(path.clone(), (Some(document.bytes()?), mode));
            }
            if path.exists() || changes.contains_key(&path) {
                let bytes = if let Some((Some(data), _)) = changes.get(&path) {
                    data.clone()
                } else {
                    fs::read(&path)?
                };
                file_states.insert(relative.into(), record(&bytes, mode));
            }
            if !wanted.is_empty() || previous.is_some() || adopt.contains(target) {
                next_targets.insert(
                    target.clone(),
                    TargetState {
                        servers: wanted.iter().map(|(n, v)| (n.clone(), v.clone())).collect(),
                    },
                );
            }
        }
        if targets.contains("opencode") {
            for relative in [".opencode/opencode.json", ".opencode/opencode.jsonc"] {
                let path = safe_path(&self.root, relative)?;
                observed.insert(path.clone(), optional_record(&path)?);
                if path.exists() && !object(&Native::read(&path)?.servers("mcp")?)?.is_empty() {
                    return Err(Error::msg(format!(
                        "{relative} overrides project MCP; consolidate its servers into opencode.json(c)"
                    )));
                }
            }
        }
        let skill_paths: BTreeSet<_> = self
            .state
            .skills
            .keys()
            .chain(desired_skills.keys())
            .cloned()
            .collect();
        let mut next_skills = Records::new();
        for relative in skill_paths {
            let path = safe_path(&self.root, &relative)?;
            let actual = optional_record(&path)?;
            observed.insert(path.clone(), actual.clone());
            if let Some(actual) = &actual {
                match self.state.skills.get(&relative) {
                    None => return Err(Error::msg(format!("unowned skill file: {relative}"))),
                    Some(old) if old != actual => {
                        return Err(Error::msg(format!(
                            "locally edited generated file: {relative}"
                        )));
                    }
                    _ => {}
                }
            }
            if let Some((content, mode)) = desired_skills.get(&relative) {
                let rec = record(content, *mode);
                file_states.insert(relative.clone(), rec.clone());
                next_skills.insert(relative, rec.clone());
                if actual.as_ref() != Some(&rec) {
                    changes.insert(path, (Some(content.clone()), *mode));
                }
            } else if actual.is_some() {
                changes.insert(path, (None, 0o644));
            }
        }
        let next_state = State {
            version: 1,
            signature: Some(signature.clone()),
            targets: next_targets,
            skills: next_skills,
            files: file_states,
        };
        for (path, (content, _)) in &changes {
            let relative = path.strip_prefix(&self.root).map_err(Error::msg)?.display();
            writeln!(
                out,
                "{} {relative}",
                if content.is_none() {
                    "delete"
                } else if path.exists() {
                    "update"
                } else {
                    "create"
                }
            )?;
            if dry_run {
                writeln!(out, "--- {relative}\n+++ {relative}")?;
                if path.exists() {
                    for line in String::from_utf8_lossy(&fs::read(path)?).lines() {
                        writeln!(out, "-{line}")?;
                    }
                }
                if let Some(data) = content {
                    for line in String::from_utf8_lossy(data).lines() {
                        writeln!(out, "+{line}")?;
                    }
                }
            }
        }
        if changes.is_empty() {
            writeln!(out, "Generated files are up to date.")?;
        }
        if !dry_run {
            let (exclude, _) = exclusion_change(&self.root, None)?;
            let _guard = lock(&exclude)?;
            verify_observed(&observed)?;
            // Recheck paths, library and selection inputs after reporting the preview.
            for path in observed.keys() {
                safe_path(
                    &self.root,
                    path.strip_prefix(&self.root)
                        .map_err(Error::msg)?
                        .to_str()
                        .ok_or_else(|| Error::msg("non-UTF-8 output path"))?,
                )?;
            }
            library_skills(&self.library, &self.catalog)?;
            if signature != self.signature()? {
                return Err(Error::msg("library changed while preparing sync"));
            }
            let generated = if self.excluded() {
                untracked(&self.root, next_state.files.keys())?
            } else {
                Vec::new()
            };
            let (exclude, content) = exclusion_change(&self.root, Some(&generated))?;
            if !exclude.exists() || fs::read(&exclude)? != content {
                changes.insert(exclude, (Some(content), 0o644));
            }
            if next_state != self.state {
                changes.insert(
                    safe_path(&self.root, STATE)?,
                    (Some(json_bytes(&next_state)?), 0o600),
                );
            }
            apply_changes(&changes)?;
            self.state = next_state;
            let state_path = safe_path(&self.root, STATE)?;
            self.inputs
                .insert(state_path.clone(), optional_record(&state_path)?);
        }
        Ok(!changes.is_empty())
    }
}
use std::os::unix::fs::PermissionsExt;
/// Paths Git does not track; excluding a tracked file would have no effect.
fn untracked<'p>(
    root: &Path,
    paths: impl Iterator<Item = &'p String>,
) -> Result<Vec<String>, Error> {
    let paths: Vec<&str> = paths.map(String::as_str).collect();
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = vec!["ls-files", "--"];
    args.extend(&paths);
    let tracked = run_command("git", &args, root)?;
    let tracked: BTreeSet<_> = tracked.lines().collect();
    Ok(paths
        .into_iter()
        .filter(|p| !tracked.contains(p))
        .map(str::to_owned)
        .collect())
}
fn verify_observed(observed: &BTreeMap<PathBuf, Option<Record>>) -> Result<(), Error> {
    for (path, expected) in observed {
        if &optional_record(path)? != expected {
            return Err(Error::msg(format!(
                "file changed while preparing sync: {}",
                path.display()
            )));
        }
    }
    Ok(())
}
