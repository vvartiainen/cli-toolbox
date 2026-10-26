use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Component;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub sha256: String,
    pub executable: bool,
}
pub(super) type Records = BTreeMap<String, Record>;
pub(super) type Content = (Option<Vec<u8>>, u32);
pub(super) type Changes = BTreeMap<PathBuf, Content>;

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn json_bytes(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    let mut data = serde_json::to_vec_pretty(value).map_err(Error::msg)?;
    data.push(b'\n');
    Ok(data)
}
pub(super) fn write_json(path: &Path, value: &impl Serialize) -> Result<(), Error> {
    write_file(path, &json_bytes(value)?, 0o600)
}
pub(super) fn load_json(path: &Path, default: Value) -> Result<Value, Error> {
    match fs::read_to_string(path) {
        Ok(text) => parse_json(&text)
            .map_err(|e| Error::msg(format!("invalid JSON in {}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(default),
        Err(e) => Err(e.into()),
    }
}
pub(super) fn safe_path(root: &Path, relative: &str) -> Result<PathBuf, Error> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(Error::msg(format!(
            "unsafe relative path: {}",
            relative.display()
        )));
    }
    reject_symlink(root)?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part);
        reject_symlink(&current)?;
        if current != root.join(relative) && current.exists() && !current.is_dir() {
            return Err(Error::msg(format!(
                "expected a directory: {}",
                current.display()
            )));
        }
    }
    Ok(current)
}
pub(super) fn reject_symlink(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            Err(Error::msg(format!("refusing symlink: {}", path.display())))
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub(super) fn record(bytes: &[u8], mode: u32) -> Record {
    Record {
        sha256: digest(bytes),
        executable: mode & 0o111 != 0,
    }
}
pub(super) fn file_record(path: &Path) -> Result<Record, Error> {
    reject_symlink(path)?;
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(Error::msg(format!(
            "expected a regular file: {}",
            path.display()
        )));
    }
    Ok(record(&fs::read(path)?, metadata.permissions().mode()))
}
pub(super) fn optional_record(path: &Path) -> Result<Option<Record>, Error> {
    reject_symlink(path)?;
    if path.exists() {
        Ok(Some(file_record(path)?))
    } else {
        Ok(None)
    }
}
pub(super) fn tree_records(root: &Path) -> Result<Records, Error> {
    fn visit(root: &Path, dir: &Path, records: &mut Records) -> Result<(), Error> {
        reject_symlink(dir)?;
        if !dir.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            reject_symlink(&path)?;
            if entry.file_type()?.is_dir() {
                visit(root, &path, records)?;
            } else {
                let relative = path
                    .strip_prefix(root)
                    .map_err(Error::msg)?
                    .to_str()
                    .ok_or_else(|| Error::msg("non-UTF-8 library or generated path"))?
                    .to_owned();
                records.insert(relative, file_record(&path)?);
            }
        }
        Ok(())
    }
    let mut records = Records::new();
    visit(root, root, &mut records)?;
    Ok(records)
}
pub(super) fn copy_tree(source: &Path, output: &Path) -> Result<(), Error> {
    for (relative, rec) in tree_records(source)? {
        write_file(
            &safe_path(output, &relative)?,
            &fs::read(source.join(relative))?,
            if rec.executable { 0o755 } else { 0o644 },
        )?;
    }
    Ok(())
}
pub(super) fn write_file(path: &Path, data: &[u8], mode: u32) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::msg("file has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(data)?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(Error::msg)?;
    Ok(())
}
fn original(path: &Path) -> Result<Content, Error> {
    reject_symlink(path)?;
    if path.exists() {
        file_record(path)?;
        Ok((
            Some(fs::read(path)?),
            fs::metadata(path)?.permissions().mode() & 0o777,
        ))
    } else {
        Ok((None, 0o600))
    }
}
fn apply_one(path: &Path, content: &Content) -> Result<(), Error> {
    if let Some(data) = &content.0 {
        write_file(path, data, content.1)
    } else if path.exists() {
        fs::remove_file(path)?;
        // Drop directories the file leaves empty; the first nonempty one,
        // at the latest the repository root, stops this.
        let mut parent = path.parent();
        while let Some(dir) = parent.filter(|dir| fs::remove_dir(dir).is_ok()) {
            parent = dir.parent();
        }
        Ok(())
    } else {
        Ok(())
    }
}
pub(super) fn apply_changes(changes: &Changes) -> Result<(), Error> {
    apply_changes_with(changes, apply_one)
}
pub(super) fn apply_changes_with(
    changes: &Changes,
    mut apply: impl FnMut(&Path, &Content) -> Result<(), Error>,
) -> Result<(), Error> {
    let originals: BTreeMap<_, _> = changes
        .keys()
        .map(|p| Ok((p.clone(), original(p)?)))
        .collect::<Result<_, Error>>()?;
    let mut completed = Vec::new();
    let mut created_dirs = BTreeSet::new();
    for (path, content) in changes {
        let mut parent = path.parent();
        while let Some(dir) = parent {
            if dir.exists() {
                break;
            }
            created_dirs.insert(dir.to_path_buf());
            parent = dir.parent();
        }
        // Include the failing write: an error may occur after its rename.
        completed.push(path);
        if let Err(error) = apply(path, content) {
            let mut rollback_error = None;
            for path in completed.into_iter().rev() {
                if let Err(e) = apply_one(path, &originals[path]) {
                    rollback_error = Some(e);
                }
            }
            for dir in created_dirs.into_iter().rev() {
                let _ = fs::remove_dir(dir);
            }
            return Err(match rollback_error {
                Some(e) => Error::msg(format!("{error}; rollback failed: {e}")),
                None => error,
            });
        }
    }
    Ok(())
}

// Lock outside the repository so previews do not create project or Git files.
pub(super) fn lock(identity: &Path) -> Result<File, Error> {
    let name = format!(
        "cli-toolbox-agents-{}-{}.lock",
        unsafe { libc::getuid() },
        digest(identity.as_os_str().as_encoded_bytes())
    );
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(std::env::temp_dir().join(name))?;
    if !file.metadata()?.is_file() {
        return Err(Error::msg("expected a regular lock file"));
    }
    file.lock()?;
    Ok(file)
}
