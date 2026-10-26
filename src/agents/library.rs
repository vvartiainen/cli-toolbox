use super::*;
use serde::{Deserialize, Serialize};

const SKILLS: &str = ".rulesync/skills/.curated";
const LOCKFILES: [&str; 2] = ["rulesync.lock", "rulesync-npm.lock.json"];
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    sources: String,
    files: Records,
    lockfiles: Records,
}
fn source_signature(catalog: &Value) -> Result<String, Error> {
    Ok(digest(&json_bytes(&catalog["sources"])?))
}
pub(super) fn write_receipt(library: &Path, catalog: &Value) -> Result<(), Error> {
    let files = tree_records(&safe_path(library, SKILLS)?)?;
    let mut lockfiles = Records::new();
    for name in LOCKFILES {
        if let Some(record) = optional_record(&safe_path(library, name)?)? {
            lockfiles.insert(name.into(), record);
        }
    }
    write_json(
        &library.join("receipt.json"),
        &Receipt {
            sources: source_signature(catalog)?,
            files,
            lockfiles,
        },
    )
}
pub(super) fn library_skills(
    library: &Path,
    catalog: &Value,
) -> Result<BTreeMap<String, PathBuf>, Error> {
    let mut skills = installed_skills(library, catalog)?;
    // Local skills come from immutable paths (e.g. the Nix store), so they
    // need no install step or receipt.
    if let Some(local) = catalog.get("localSkills") {
        for (name, path) in object(local)? {
            let path = PathBuf::from(path.as_str().unwrap());
            if !path.join("SKILL.md").is_file() {
                return Err(Error::msg(format!(
                    "local skill {name} has no SKILL.md: {}",
                    path.display()
                )));
            }
            if skills.insert(name.clone(), path).is_some() {
                return Err(Error::msg(format!(
                    "local skill {name} clashes with an installed skill"
                )));
            }
        }
    }
    Ok(skills)
}
fn installed_skills(library: &Path, catalog: &Value) -> Result<BTreeMap<String, PathBuf>, Error> {
    let receipt = load_json(&safe_path(library, "receipt.json")?, Value::Null)?;
    if receipt.is_null() && catalog["sources"].as_array().is_some_and(Vec::is_empty) {
        return Ok(BTreeMap::new());
    }
    if receipt.is_null() || receipt["sources"] != source_signature(catalog)? {
        return Err(Error::msg(
            "skill sources changed or are not installed; run ct agents library install",
        ));
    }
    let receipt: Receipt = serde_json::from_value(receipt).map_err(Error::msg)?;
    let root = safe_path(library, SKILLS)?;
    if tree_records(&root)? != receipt.files {
        return Err(Error::msg(
            "installed library was edited; run ct agents library install to restore locked skills",
        ));
    }
    for (name, record) in receipt.lockfiles {
        if !LOCKFILES.contains(&name.as_str())
            || optional_record(&safe_path(library, &name)?)? != Some(record)
        {
            return Err(Error::msg(
                "library lockfile changed; run ct agents library install",
            ));
        }
    }
    let mut skills = BTreeMap::new();
    if root.exists() {
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| Error::msg("non-UTF-8 skill name"))?;
            if entry.file_type()?.is_dir() && entry.path().join("SKILL.md").is_file() {
                if !valid_name(&name) {
                    return Err(Error::msg(format!("unsafe skill name: {name}")));
                }
                skills.insert(name, entry.path());
            }
        }
    }
    Ok(skills)
}
pub(super) fn install_library(
    library: &Path,
    catalog: &Value,
    backend: &dyn Backend,
    update: bool,
) -> Result<(), Error> {
    validate_catalog(catalog)?;
    reject_symlink(library)?;
    let parent = library
        .parent()
        .ok_or_else(|| Error::msg("library has no parent"))?;
    let _guard = lock(library)?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::Builder::new()
        .prefix(".ct-library-")
        .tempdir_in(parent)?;
    let stage = temporary.path().join("library");
    fs::create_dir(&stage)?;
    for name in LOCKFILES {
        let source = safe_path(library, name)?;
        if source.exists() {
            file_record(&source)?;
            fs::copy(&source, stage.join(name))?;
        }
    }
    write_json(
        &stage.join("rulesync.jsonc"),
        // Rulesync requires a target, though installing only fills the library.
        &json!({"targets":["claudecode"],"features":["skills"],"delete":false,"global":false,"sources":catalog["sources"]}),
    )?;
    if !catalog["sources"].as_array().unwrap().is_empty() {
        backend.install(&stage, update)?;
        let root = safe_path(&stage, SKILLS)?;
        tree_records(&root)?;
        let installed: BTreeSet<String> = if root.exists() {
            fs::read_dir(&root)?
                .map(|e| {
                    let e = e?;
                    Ok(
                        if e.file_type()?.is_dir() && e.path().join("SKILL.md").is_file() {
                            e.file_name().to_str().map(str::to_owned)
                        } else {
                            None
                        },
                    )
                })
                .collect::<Result<Vec<_>, Error>>()?
                .into_iter()
                .flatten()
                .collect()
        } else {
            BTreeSet::new()
        };
        if installed.is_empty() {
            return Err(Error::msg(
                "Rulesync installed no skills from the declared sources",
            ));
        }
        for source in catalog["sources"].as_array().unwrap() {
            if let Some(selected) = source.get("skills").and_then(Value::as_array) {
                for name in selected {
                    let name = name
                        .as_str()
                        .ok_or_else(|| Error::msg("invalid source skill selection"))?;
                    if name != "*" && !installed.contains(name) {
                        return Err(Error::msg(format!(
                            "Rulesync did not install selected skill: {name}"
                        )));
                    }
                }
            }
        }
    } else {
        write_json(
            &stage.join("rulesync.lock"),
            &json!({"lockfileVersion":1,"sources":{}}),
        )?;
    }
    write_receipt(&stage, catalog)?;
    // Validate the full staged installation before replacing the working library.
    library_skills(&stage, catalog)?;
    promote_library(temporary, library, |from, to| fs::rename(from, to))
}

pub(super) fn promote_library(
    temporary: tempfile::TempDir,
    library: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), Error> {
    let stage = temporary.path().join("library");
    let backup = temporary.path().join("previous");
    if library.exists() {
        rename(library, &backup)?;
    }
    if let Err(error) = rename(&stage, library) {
        if backup.exists()
            && let Err(restore_error) = rename(&backup, library)
        {
            // Retain the only working copy even if the filesystem also refuses
            // rollback. TempDir cleanup must never discard this backup.
            let saved = temporary.keep().join("previous");
            return Err(Error::msg(format!(
                "{error}; restoring library failed: {restore_error}; previous installation retained at {}",
                saved.display()
            )));
        }
        return Err(error.into());
    }
    Ok(())
}
