use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::Error;
use serde::{Deserialize, Serialize};
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

/// The choices of the working directory's repository and whether its files
/// match them, for the TUI.
pub(crate) fn overview() -> Result<(Vec<Entry>, bool), Error> {
    Environment::discover()?.with_project(|project| Ok((project.overview(), project.is_synced()?)))
}
/// Turn one target, MCP server or skill on or off for the working directory's
/// repository, without syncing.
pub(crate) fn set_choice(group: &str, name: &str, enabled: bool) -> Result<(), Error> {
    Environment::discover()?.with_project(|project| project.set_choice(group, name, enabled))
}
/// Sync, or preview with `dry_run`, the working directory's repository.
pub(crate) fn sync(dry_run: bool, out: &mut dyn Write) -> Result<(), Error> {
    Environment::discover()?.with_project(|project| project.sync(dry_run, false, out))
}

/// One target, catalog MCP server or skill and whether the repository uses it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub group: String,
    pub name: String,
    pub enabled: bool,
}

/// What sync wrote in a checkout: each target's MCP servers in its own format,
/// and every skill file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lock {
    version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    mcp: BTreeMap<String, serde_json::Map<String, Value>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    skills: Records,
}
impl Lock {
    fn is_empty(&self) -> bool {
        self.mcp.is_empty() && self.skills.is_empty()
    }
}

/// The changes a sync makes in the checkout, and the lock after it.
struct Plan {
    changes: Changes,
    lock: Lock,
}

/// A repository's agent tools: its choices from the catalog, kept outside the
/// checkout, and the agent files sync writes into it.
pub(crate) struct Project {
    root: PathBuf,
    /// The repository's choices, shared by its worktrees.
    choices_path: PathBuf,
    /// What sync wrote in this checkout.
    lock_path: PathBuf,
    catalog: Value,
    skills: BTreeMap<String, PathBuf>,
    choices: Value,
    /// Whether the choices were worked out from the agent files because none
    /// were saved, as in a fresh clone or on another machine.
    inferred: bool,
    /// None when this checkout has no lock yet: catalog-named entries in the
    /// agent files are then taken over.
    lock: Option<Lock>,
}
impl Project {
    fn new(root: &Path, catalog: Value, library: &Path, state_home: &Path) -> Result<Self, Error> {
        validate_catalog(&catalog)?;
        let root = root.canonicalize()?;
        let state = state_dir(state_home, &root)?;
        let lock_path = lock_path(&state, &root);
        let skills = library_skills(library, &catalog)?;
        let choices_path = state.join(CHOICES);
        let choices = load_json(&choices_path, Value::Null)?;
        if !choices.is_null() && choices.get("version") != Some(&json!(1)) {
            return Err(Error::msg("unsupported choices version"));
        }
        let lock = match load_json(&lock_path, Value::Null)? {
            Value::Null => None,
            value => {
                let lock: Lock = serde_json::from_value(value)
                    .map_err(|e| Error::msg(format!("invalid lock: {e}")))?;
                if lock.version != 1 {
                    return Err(Error::msg("unsupported lock version"));
                }
                for path in lock.skills.keys() {
                    safe_path(&root, path)?;
                }
                if lock.mcp.keys().any(|t| !TARGETS.contains(&t.as_str())) {
                    return Err(Error::msg("unknown target in lock"));
                }
                Some(lock)
            }
        };
        let mut project = Self {
            root,
            choices_path,
            lock_path,
            catalog,
            skills,
            inferred: choices.is_null(),
            choices,
            lock,
        };
        if project.inferred {
            project.choices = project.infer_choices()?;
        }
        Ok(project)
    }
    /// Choices from the agent files: catalog-named servers in a target's MCP
    /// file, and catalog-named skill folders. Only `.claude/skills` tells which
    /// agent reads a skill, so targets that have only skills are not found.
    fn infer_choices(&self) -> Result<Value, Error> {
        let mut choices = json!({"version":1});
        let mut on = |group: &str, name: &str| {
            choices[group][name] = json!("enable");
        };
        for target in TARGETS {
            let (relative, key) = self.mcp_file(target)?;
            let path = safe_path(&self.root, relative)?;
            if !path.exists() {
                continue;
            }
            let servers = Native::read(&path)?.servers(key)?;
            for name in self.names("mcp") {
                if servers.get(&name).is_some() {
                    on("targets", target);
                    on("mcp", &name);
                }
            }
        }
        for dir in [".claude/skills", ".agents/skills"] {
            for name in self.skills.keys() {
                if safe_path(&self.root, &format!("{dir}/{name}"))?.is_dir() {
                    on("skills", name);
                    if dir == ".claude/skills" {
                        on("targets", "claude");
                    }
                }
            }
        }
        Ok(choices)
    }
    fn names(&self, group: &str) -> Vec<String> {
        match group {
            "targets" => TARGETS.map(String::from).to_vec(),
            "mcp" => self.catalog["mcp"]
                .as_object()
                .map(|servers| servers.keys().cloned().collect())
                .unwrap_or_default(),
            _ => self.skills.keys().cloned().collect(),
        }
    }
    /// Whether the repository uses it. Choices of names that no longer exist
    /// are ignored.
    fn enabled(&self, group: &str, name: &str) -> bool {
        self.choices[group][name] == "enable"
    }
    pub fn overview(&self) -> Vec<Entry> {
        GROUPS
            .into_iter()
            .flat_map(|group| {
                self.names(group).into_iter().map(move |name| Entry {
                    group: group.into(),
                    enabled: self.enabled(group, &name),
                    name,
                })
            })
            .collect()
    }
    pub fn set_choice(&mut self, group: &str, name: &str, enabled: bool) -> Result<(), Error> {
        if !GROUPS.contains(&group) {
            return Err(Error::msg(format!("unknown group: {group}")));
        }
        if !self.names(group).iter().any(|n| n == name) {
            return Err(Error::msg(format!("unknown {group} entry: {name}")));
        }
        let mut choices = self.choices.clone();
        if !choices[group].is_object() {
            choices[group] = json!({});
        }
        let values = choices[group].as_object_mut().unwrap();
        if enabled {
            values.insert(name.into(), json!("enable"));
        } else {
            values.remove(name);
        }
        write_json(&self.choices_path, &choices)?;
        self.choices = choices;
        self.inferred = false;
        Ok(())
    }
    pub fn status(&self, out: &mut dyn Write) -> Result<(), Error> {
        if self.inferred {
            writeln!(
                out,
                "No saved choices; these were worked out from the agent files."
            )?;
        }
        for entry in self.overview() {
            writeln!(
                out,
                "{} / {}: {}",
                entry.group,
                entry.name,
                if entry.enabled { "on" } else { "off" }
            )?;
        }
        writeln!(
            out,
            "{}",
            if self.is_synced()? {
                "Files are up to date."
            } else {
                "Pending changes; preview with ct agents sync --dry-run."
            }
        )?;
        Ok(())
    }
    /// Toggle choices in a picker until it is closed, then sync.
    pub fn interactive(&mut self, out: &mut dyn Write) -> Result<(), Error> {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            return Err(Error::msg(
                "interactive selection requires a terminal; use ct agents set",
            ));
        }
        loop {
            let entries = self.overview();
            let mut items: Vec<_> = entries
                .iter()
                .map(|e| {
                    let state = if e.enabled { "[on] " } else { "[off]" };
                    format!("{state} {} / {}", e.group, e.name)
                })
                .collect();
            items.push("Sync and quit".into());
            let Some(index) = crate::select::pick("toggle > ", &items, "~100%")? else {
                return Ok(());
            };
            let Some(entry) = entries.get(index) else {
                return self.sync(false, false, out);
            };
            self.set_choice(&entry.group, &entry.name, !entry.enabled)?;
        }
    }
    pub fn is_synced(&self) -> Result<bool, Error> {
        Ok(self.plan(false)?.changes.is_empty())
    }
    /// Write the agent files. `replace` takes over catalog-named entries that
    /// differ from what sync would write, instead of stopping.
    pub fn sync(&mut self, dry_run: bool, replace: bool, out: &mut dyn Write) -> Result<(), Error> {
        let plan = self.plan(replace)?;
        if plan.changes.is_empty() {
            writeln!(out, "Up to date.")?;
        }
        for (path, (content, _)) in &plan.changes {
            let relative = path.strip_prefix(&self.root).map_err(Error::msg)?;
            let action = match (content, path.exists()) {
                (None, _) => "delete",
                (Some(_), true) => "update",
                (Some(_), false) => "create",
            };
            writeln!(out, "{action} {}", relative.display())?;
        }
        if dry_run {
            return Ok(());
        }
        let mut changes = plan.changes;
        let lock = (!plan.lock.is_empty())
            .then(|| json_bytes(&plan.lock))
            .transpose()?;
        changes.insert(self.lock_path.clone(), (lock, 0o600));
        if self.inferred {
            changes.insert(
                self.choices_path.clone(),
                (Some(json_bytes(&self.choices)?), 0o600),
            );
        }
        apply_changes(&changes)?;
        self.lock = Some(plan.lock);
        self.inferred = false;
        Ok(())
    }
    /// A target's MCP file: the first candidate that exists, else the last.
    fn mcp_file(&self, target: &str) -> Result<(&'static str, &'static str), Error> {
        let (candidates, key) = mcp_files(target);
        let existing: Vec<_> = candidates
            .iter()
            .filter(|relative| self.root.join(relative).exists())
            .collect();
        if existing.len() > 1 {
            return Err(Error::msg(format!(
                "both {} exist; keep one",
                candidates.join(" and ")
            )));
        }
        let relative = existing
            .first()
            .copied()
            .unwrap_or(candidates.last().unwrap());
        Ok((relative, key))
    }
    /// Compare the files with the choices. What sync did not write, or what
    /// changed since, is not overwritten unless `replace`: that is an error to
    /// resolve.
    fn plan(&self, replace: bool) -> Result<Plan, Error> {
        let targets: BTreeSet<&str> = TARGETS
            .into_iter()
            .filter(|t| self.enabled("targets", t))
            .collect();
        let servers: Vec<(&String, &Value)> = object(&self.catalog["mcp"])?
            .iter()
            .filter(|(name, _)| self.enabled("mcp", name))
            .collect();
        if !targets.is_empty() {
            for (name, server) in &servers {
                portable(name, server)?;
            }
        }
        let mut changes = Changes::new();
        let mut lock = Lock {
            version: 1,
            ..Lock::default()
        };
        for target in TARGETS {
            let wanted: serde_json::Map<String, Value> = if targets.contains(target) {
                servers
                    .iter()
                    .map(|(name, server)| ((*name).clone(), native_server(target, server)))
                    .collect()
            } else {
                serde_json::Map::new()
            };
            self.plan_mcp(target, &wanted, replace, &mut changes)?;
            if !wanted.is_empty() {
                lock.mcp.insert(target.into(), wanted);
            }
        }
        lock.skills = self.plan_skills(&targets, replace, &mut changes)?;
        Ok(Plan { changes, lock })
    }
    fn plan_mcp(
        &self,
        target: &str,
        wanted: &serde_json::Map<String, Value>,
        replace: bool,
        changes: &mut Changes,
    ) -> Result<(), Error> {
        let (relative, key) = self.mcp_file(target)?;
        let path = safe_path(&self.root, relative)?;
        let recorded = self
            .lock
            .as_ref()
            .map(|lock| lock.mcp.get(target).cloned().unwrap_or_default());
        if wanted.is_empty() && (recorded.as_ref().is_some_and(|r| r.is_empty()) || !path.exists())
        {
            return Ok(());
        }
        let mut document = Native::read(&path)?;
        let current_value = document.servers(key)?;
        let current = object(&current_value)?;
        let differs = |name: &str| {
            format!(
                "MCP server {name} in {relative} differs from what ct agents wrote; \
                 restore or remove it, or sync with --replace"
            )
        };
        // Without a lock, the catalog's servers are taken to be ours.
        let owned = recorded.unwrap_or_else(|| {
            current
                .iter()
                .filter(|(name, _)| self.catalog["mcp"].get(name.as_str()).is_some())
                .map(|(name, entry)| (name.clone(), entry.clone()))
                .collect()
        });
        for (name, entry) in &owned {
            // Without a lock, ours is what sync would write from the catalog.
            let expected = if self.lock.is_some() {
                entry.clone()
            } else {
                native_server(target, &self.catalog["mcp"][name])
            };
            if current.get(name).is_some_and(|actual| *actual != expected) && !replace {
                return Err(Error::msg(differs(name)));
            }
        }
        for (name, entry) in wanted {
            // An identical server is adopted; anything else is someone else's.
            if !owned.contains_key(name)
                && current.get(name).is_some_and(|v| v != entry)
                && !replace
            {
                return Err(Error::msg(format!(
                    "{relative} already has another MCP server named {name}; \
                     remove or rename it, or sync with --replace"
                )));
            }
        }
        let mut combined = current.clone();
        for name in owned.keys() {
            combined.remove(name);
        }
        combined.extend(wanted.clone());
        if &combined != current {
            document.set_servers(key, Value::Object(combined))?;
            // A file left with nothing in it is removed.
            let data = (!document.is_empty())
                .then(|| document.bytes())
                .transpose()?;
            let mode = if path.exists() {
                fs::metadata(&path)?.permissions().mode() & 0o777
            } else {
                0o644
            };
            changes.insert(path, (data, mode));
        }
        Ok(())
    }
    /// Copy the chosen skills into the folders the targets read.
    fn plan_skills(
        &self,
        targets: &BTreeSet<&str>,
        replace: bool,
        changes: &mut Changes,
    ) -> Result<Records, Error> {
        let mut desired = BTreeMap::new();
        for dir in skill_dirs(targets) {
            for (name, source) in &self.skills {
                if !self.enabled("skills", name) {
                    continue;
                }
                for (file, rec) in tree_records(source)? {
                    let relative = format!("{dir}/{name}/{file}");
                    desired.insert(relative, (source.join(&file), rec));
                }
            }
        }
        let owned = match &self.lock {
            Some(lock) => lock.skills.clone(),
            None => self.found_skills(&desired, replace)?,
        };
        let paths: BTreeSet<_> = owned.keys().chain(desired.keys()).cloned().collect();
        let mut records = Records::new();
        for relative in paths {
            let path = safe_path(&self.root, &relative)?;
            let actual = optional_record(&path)?;
            let wanted = desired.get(&relative);
            if !replace {
                match (owned.get(&relative), &actual) {
                    (Some(old), Some(actual)) if old != actual => {
                        return Err(Error::msg(format!(
                            "skill file {relative} differs from what ct agents wrote; \
                             restore or remove it, or sync with --replace"
                        )));
                    }
                    (None, Some(actual)) if wanted.is_some_and(|(_, rec)| rec != actual) => {
                        return Err(Error::msg(format!(
                            "skill file {relative} exists and was not written by ct agents; \
                             remove it, or sync with --replace"
                        )));
                    }
                    _ => {}
                }
            }
            if let Some((source, rec)) = wanted {
                if actual.as_ref() != Some(rec) {
                    let mode = if rec.executable { 0o755 } else { 0o644 };
                    changes.insert(path, (Some(fs::read(source)?), mode));
                }
                records.insert(relative, rec.clone());
            } else if actual.is_some() {
                changes.insert(path, (None, 0o644));
            }
        }
        Ok(records)
    }
    /// Without a lock, the files of catalog-named skill folders are taken to be
    /// ours. A folder that differs from what sync would write is only taken
    /// over with `replace`.
    fn found_skills(
        &self,
        desired: &BTreeMap<String, (PathBuf, Record)>,
        replace: bool,
    ) -> Result<Records, Error> {
        let mut found = Records::new();
        for dir in [".claude/skills", ".agents/skills"] {
            for name in self.skills.keys() {
                let folder = format!("{dir}/{name}");
                let path = safe_path(&self.root, &folder)?;
                if !path.is_dir() {
                    continue;
                }
                let files: Records = tree_records(&path)?
                    .into_iter()
                    .map(|(file, rec)| (format!("{folder}/{file}"), rec))
                    .collect();
                let expected: Records = desired
                    .iter()
                    .filter(|(relative, _)| relative.starts_with(&format!("{folder}/")))
                    .map(|(relative, (_, rec))| (relative.clone(), rec.clone()))
                    .collect();
                if !expected.is_empty() && files != expected && !replace {
                    return Err(Error::msg(format!(
                        "skill folder {folder} differs from the catalog's; \
                         sync with --replace to replace it"
                    )));
                }
                found.extend(files);
            }
        }
        Ok(found)
    }
}
