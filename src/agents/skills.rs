use super::*;

/// A catalog skill and the differences between its project copies and source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SkillStatus {
    pub name: String,
    pub enabled: bool,
    pub details: Vec<String>,
    pub differs: bool,
}

pub(super) fn belongs_to(path: &str, name: &str) -> bool {
    [".claude/skills", ".agents/skills"]
        .iter()
        .any(|dir| path.starts_with(&format!("{dir}/{name}/")))
}

impl Project {
    pub fn skill_statuses(&self) -> Result<Vec<SkillStatus>, Error> {
        let targets = TARGETS
            .into_iter()
            .filter(|t| self.enabled("targets", t))
            .collect();
        let desired_dirs = skill_dirs(&targets);
        self.skills
            .iter()
            .map(|(name, source)| {
                let enabled = self.enabled("skills", name);
                let expected = tree_records(source)?;
                let mut details = Vec::new();
                let mut differs = false;
                for dir in [".claude/skills", ".agents/skills"] {
                    let folder = format!("{dir}/{name}");
                    let path = safe_path(&self.root, &folder)?;
                    let wanted = enabled && desired_dirs.contains(&dir);
                    if !path.exists() && !wanted {
                        continue;
                    }
                    let actual = tree_records(&path)?;
                    if !wanted {
                        details.push(format!("pending removal {folder}"));
                        differs = true;
                    }
                    let files: BTreeSet<_> = actual.keys().chain(expected.keys()).collect();
                    for file in files {
                        let state = match (actual.get(file), expected.get(file)) {
                            (None, Some(_)) => "missing",
                            (Some(_), None) => "extra",
                            (Some(actual), Some(expected)) if actual != expected => "changed",
                            _ => continue,
                        };
                        differs = true;
                        details.push(format!("{state} {folder}/{file}"));
                    }
                    if wanted && actual == expected {
                        details.push(format!("up to date {folder}"));
                    }
                }
                if details.is_empty() {
                    details.push(
                        if enabled {
                            "No targets enabled."
                        } else {
                            "Not installed in this checkout."
                        }
                        .into(),
                    );
                }
                Ok(SkillStatus {
                    name: name.clone(),
                    enabled,
                    details,
                    differs,
                })
            })
            .collect()
    }

    pub fn list_skills(&self, out: &mut dyn Write) -> Result<(), Error> {
        for skill in self.skill_statuses()? {
            writeln!(
                out,
                "{}: {}",
                skill.name,
                if skill.enabled { "on" } else { "off" }
            )?;
            for detail in skill.details {
                writeln!(out, "  {detail}")?;
            }
        }
        Ok(())
    }

    /// Sync only this skill, preserving other skills' and MCP servers' records.
    pub fn sync_skill(
        &mut self,
        name: &str,
        dry_run: bool,
        replace: bool,
        out: &mut dyn Write,
    ) -> Result<(), Error> {
        if !self.skills.contains_key(name) {
            return Err(Error::msg(format!("unknown skill: {name}")));
        }
        let targets = TARGETS
            .into_iter()
            .filter(|t| self.enabled("targets", t))
            .collect();
        let mut changes = Changes::new();
        let mut conflicts = Vec::new();
        let skills = self.plan_skills(&targets, Some(name), &mut changes, &mut conflicts)?;
        let mut lock = self.lock.clone().unwrap_or_else(|| Lock {
            version: 1,
            partial: true,
            ..Lock::default()
        });
        lock.skills.retain(|path, _| !belongs_to(path, name));
        lock.skills.extend(skills);
        self.apply_plan(
            Plan {
                changes,
                lock,
                conflicts,
            },
            dry_run,
            replace,
            out,
        )
    }
}
