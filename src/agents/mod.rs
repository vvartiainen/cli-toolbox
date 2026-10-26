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
    Environment::discover()?.with_project(|project| project.sync(dry_run, out))
}

/// One target, catalog MCP server or skill and whether the repository uses it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub group: String,
    pub name: String,
    pub enabled: bool,
}

/// What sync wrote: each target's MCP servers in its own format, and every
/// skill file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lock {
    version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    mcp: BTreeMap<String, serde_json::Map<String, Value>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    skills: Records,
}

/// The changes a sync makes, and the lock after it.
struct Plan {
    changes: Changes,
    lock: Lock,
}

/// A repository's agent tools: its choices from the catalog and the files
/// sync manages, both committed with it.
pub(crate) struct Project {
    root: PathBuf,
    catalog: Value,
    skills: BTreeMap<String, PathBuf>,
    choices: Value,
    lock: Lock,
}
impl Project {
    fn new(root: &Path, catalog: Value, library: &Path) -> Result<Self, Error> {
        validate_catalog(&catalog)?;
        let root = root.canonicalize()?;
        let skills = library_skills(library, &catalog)?;
        let choices = load_json(&safe_path(&root, CHOICES)?, json!({"version":1}))?;
        if choices.get("version") != Some(&json!(1)) {
            return Err(Error::msg("unsupported choices version"));
        }
        let lock = load_json(&safe_path(&root, LOCK)?, json!({"version":1}))?;
        let lock: Lock =
            serde_json::from_value(lock).map_err(|e| Error::msg(format!("invalid {LOCK}: {e}")))?;
        if lock.version != 1 {
            return Err(Error::msg(format!("unsupported {LOCK} version")));
        }
        for path in lock.skills.keys() {
            safe_path(&root, path)?;
        }
        if lock.mcp.keys().any(|t| !TARGETS.contains(&t.as_str())) {
            return Err(Error::msg(format!("unknown target in {LOCK}")));
        }
        Ok(Self {
            root,
            catalog,
            skills,
            choices,
            lock,
        })
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
        let path = safe_path(&self.root, CHOICES)?;
        apply_changes(&BTreeMap::from([(
            path,
            (Some(json_bytes(&choices)?), 0o644),
        )]))?;
        self.choices = choices;
        Ok(())
    }
    pub fn status(&self, out: &mut dyn Write) -> Result<(), Error> {
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
                return self.sync(false, out);
            };
            self.set_choice(&entry.group, &entry.name, !entry.enabled)?;
        }
    }
    pub fn is_synced(&self) -> Result<bool, Error> {
        Ok(self.plan()?.changes.is_empty())
    }
    pub fn sync(&mut self, dry_run: bool, out: &mut dyn Write) -> Result<(), Error> {
        let plan = self.plan()?;
        if plan.changes.is_empty() {
            writeln!(out, "Up to date.")?;
            return Ok(());
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
        if !dry_run {
            apply_changes(&plan.changes)?;
            self.lock = plan.lock;
        }
        Ok(())
    }
    /// Compare the files with the choices. Files sync did not write, or that
    /// changed since, are never overwritten: that is an error to resolve.
    fn plan(&self) -> Result<Plan, Error> {
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
            let owned = self.lock.mcp.get(target).cloned().unwrap_or_default();
            self.plan_mcp(target, &wanted, &owned, &mut changes)?;
            if !wanted.is_empty() {
                lock.mcp.insert(target.into(), wanted);
            }
        }
        lock.skills = self.plan_skills(&targets, &mut changes)?;
        // Choices without any files need no lock.
        let lock_path = safe_path(&self.root, LOCK)?;
        let lock_bytes = (lock
            != Lock {
                version: 1,
                ..Lock::default()
            })
        .then(|| json_bytes(&lock))
        .transpose()?;
        if optional_bytes(&lock_path)? != lock_bytes {
            changes.insert(lock_path, (lock_bytes, 0o644));
        }
        Ok(Plan { changes, lock })
    }
    fn plan_mcp(
        &self,
        target: &str,
        wanted: &serde_json::Map<String, Value>,
        owned: &serde_json::Map<String, Value>,
        changes: &mut Changes,
    ) -> Result<(), Error> {
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
        let path = safe_path(&self.root, relative)?;
        if wanted.is_empty() && owned.is_empty() {
            return Ok(());
        }
        let mut document = Native::read(&path)?;
        let current_value = document.servers(key)?;
        let current = object(&current_value)?;
        for (name, entry) in owned {
            if current.get(name).is_some_and(|v| v != entry) {
                return Err(Error::msg(format!(
                    "MCP server {name} in {relative} was edited; restore or remove it"
                )));
            }
        }
        for (name, entry) in wanted {
            // An identical server is adopted; anything else is someone else's.
            if !owned.contains_key(name) && current.get(name).is_some_and(|v| v != entry) {
                return Err(Error::msg(format!(
                    "{relative} already has another MCP server named {name}; remove or rename it"
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
        let paths: BTreeSet<_> = self
            .lock
            .skills
            .keys()
            .chain(desired.keys())
            .cloned()
            .collect();
        let mut records = Records::new();
        for relative in paths {
            let path = safe_path(&self.root, &relative)?;
            let actual = optional_record(&path)?;
            let wanted = desired.get(&relative);
            match (self.lock.skills.get(&relative), &actual) {
                (Some(old), Some(actual)) if old != actual => {
                    return Err(Error::msg(format!(
                        "skill file {relative} was edited; restore or remove it"
                    )));
                }
                (None, Some(actual)) if wanted.is_some_and(|(_, rec)| rec != actual) => {
                    return Err(Error::msg(format!(
                        "skill file {relative} exists and was not written by ct agents; remove it"
                    )));
                }
                _ => {}
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
}
fn optional_bytes(path: &Path) -> Result<Option<Vec<u8>>, Error> {
    reject_symlink(path)?;
    if path.exists() {
        Ok(Some(fs::read(path)?))
    } else {
        Ok(None)
    }
}
