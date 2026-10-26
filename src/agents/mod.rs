use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::Error;
use serde_json::{Value, json};

mod backend;
mod config;
mod files;
mod launch;
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
pub(crate) use launch::{Agent, Launch};

/// The choices of the working directory's repository, for the TUI.
pub(crate) fn overview() -> Result<Vec<Entry>, Error> {
    Environment::discover()?.with_project(|project| Ok(project.overview()))
}
/// The chosen tools for `agent` in the working directory's repository; none
/// outside a repository.
pub(crate) fn launch_tools(agent: Agent) -> Result<Option<Launch>, Error> {
    if project_root(&std::env::current_dir()?).is_err() {
        return Ok(None);
    }
    Environment::discover()?
        .with_project(|project| project.launch(agent))
        .map(Some)
}
/// Turn one MCP server or skill on or off for the working directory's repository.
pub(crate) fn set_choice(group: &str, name: &str, enabled: bool) -> Result<(), Error> {
    Environment::discover()?.with_project(|project| project.set_choice(group, name, enabled))
}

/// One catalog MCP server or skill and whether this checkout uses it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub group: String,
    pub name: String,
    pub enabled: bool,
}

/// A checkout's personal choices from the catalog. They live outside the
/// checkout and reach agents only when started with `ct agents launch`.
pub(crate) struct Project {
    /// Where this checkout's choices and launch files live.
    private: PathBuf,
    catalog: Value,
    skills: BTreeMap<String, PathBuf>,
    choices: Value,
}
impl Project {
    fn new(root: &Path, catalog: Value, library: &Path, state_home: &Path) -> Result<Self, Error> {
        validate_catalog(&catalog)?;
        let private = private_dir(state_home, &root.canonicalize()?);
        let skills = library_skills(library, &catalog)?;
        let choices = load_json(&private.join(CHOICES), json!({"version":1}))?;
        if choices.get("version") != Some(&json!(1)) {
            return Err(Error::msg("unsupported choices version"));
        }
        Ok(Self {
            private,
            catalog,
            skills,
            choices,
        })
    }
    fn names(&self, group: &str) -> Vec<String> {
        match group {
            "mcp" => self.catalog["mcp"]
                .as_object()
                .map(|servers| servers.keys().cloned().collect())
                .unwrap_or_default(),
            _ => self.skills.keys().cloned().collect(),
        }
    }
    /// Whether the checkout uses it. Choices for names the catalog no longer
    /// has are ignored.
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
            return Err(Error::msg(format!("{group} {name} is not in the catalog")));
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
        write_json(&self.private.join(CHOICES), &choices)?;
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
            "Agents get these when started with ct agents launch <agent>."
        )?;
        Ok(())
    }
    /// Toggle choices in a picker until it is closed.
    pub fn interactive(&mut self, out: &mut dyn Write) -> Result<(), Error> {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            return Err(Error::msg(
                "interactive selection requires a terminal; use ct agents set",
            ));
        }
        loop {
            let entries = self.overview();
            let items: Vec<_> = entries
                .iter()
                .map(|e| {
                    let state = if e.enabled { "[on] " } else { "[off]" };
                    format!("{state} {} / {}", e.group, e.name)
                })
                .collect();
            let Some(index) = crate::select::pick("toggle > ", &items, "~100%")? else {
                writeln!(out, "Agents get these when started with ct agents launch.")?;
                return Ok(());
            };
            let entry = &entries[index];
            self.set_choice(&entry.group, &entry.name, !entry.enabled)?;
        }
    }
    /// The chosen catalog MCP servers.
    fn servers(&self) -> serde_json::Map<String, Value> {
        self.catalog["mcp"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, _)| self.enabled("mcp", name))
            .map(|(name, server)| (name.clone(), server.clone()))
            .collect()
    }
    /// The chosen skills and their directories.
    fn chosen_skills(&self) -> BTreeMap<String, PathBuf> {
        self.skills
            .iter()
            .filter(|(name, _)| self.enabled("skills", name))
            .map(|(name, path)| (name.clone(), path.clone()))
            .collect()
    }
}
