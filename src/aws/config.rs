use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub settings: BTreeMap<String, String>,
}

pub fn load_profiles(config_path: &Path, credentials_path: &Path) -> Result<Vec<Profile>, Error> {
    let mut profiles: BTreeMap<String, Profile> = BTreeMap::new();
    merge_profiles(&mut profiles, config_path, config_section_name)?;
    merge_profiles(&mut profiles, credentials_path, credentials_section_name)?;
    Ok(profiles.into_values().collect())
}

fn merge_profiles(
    dst: &mut BTreeMap<String, Profile>,
    path: &Path,
    mapper: fn(&str) -> Option<String>,
) -> Result<(), Error> {
    let sections = parse_ini_sections(path, mapper)?;
    for (name, settings) in sections {
        let profile = dst.entry(name.clone()).or_insert_with(|| Profile {
            name,
            settings: BTreeMap::new(),
        });
        profile.settings.extend(settings);
    }
    Ok(())
}

fn parse_ini_sections(
    path: &Path,
    mapper: fn(&str) -> Option<String>,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, Error> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => {
            return Err(Error::msg(format!("open {}: {err}", path.display())));
        }
    };

    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line.map_err(|err| Error::msg(format!("read {}: {err}", path.display())))?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current = mapper(name.trim());
            if let Some(current) = &current {
                sections.entry(current.clone()).or_default();
            }
            continue;
        }

        let Some(current) = &current else {
            continue;
        };
        let Some((key, value)) = split_key_value(line) else {
            continue;
        };
        sections
            .entry(current.clone())
            .or_default()
            .insert(key.to_ascii_lowercase(), value);
    }

    Ok(sections)
}

fn split_key_value(line: &str) -> Option<(String, String)> {
    let idx = line.find(['=', ':'])?;
    if idx == 0 {
        return None;
    }
    Some((
        line[..idx].trim().to_string(),
        line[idx + 1..].trim().to_string(),
    ))
}

fn config_section_name(section: &str) -> Option<String> {
    if section == "default" {
        return Some("default".into());
    }
    let rest = section.strip_prefix("profile ")?;
    let name = rest.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn credentials_section_name(section: &str) -> Option<String> {
    let section = section.trim();
    if section.is_empty() {
        None
    } else {
        Some(section.to_string())
    }
}

pub(super) fn is_sso_profile(profile: &Profile) -> bool {
    profile
        .settings
        .keys()
        .any(|key| key.starts_with("sso_") || key == "sso_session")
}

pub(super) fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".into();
    }
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::{Profile, is_sso_profile, load_profiles, shell_quote};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn load_profiles_merges_config_and_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config");
        let credentials_path = dir.path().join("credentials");

        fs::write(
            &config_path,
            r#"
[default]
region = eu-north-1

[profile sandbox]
sso_start_url = https://example.awsapps.com/start
sso_region = eu-west-1

[sso-session shared]
sso_start_url = ignored
"#,
        )
        .unwrap();
        fs::write(
            &credentials_path,
            r#"
[sandbox]
aws_access_key_id = test

[prod]
aws_secret_access_key = secret
"#,
        )
        .unwrap();

        let profiles = load_profiles(&config_path, &credentials_path).unwrap();
        let names: Vec<_> = profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect();
        assert_eq!(names, ["default", "prod", "sandbox"]);

        let sandbox = &profiles[2];
        assert_eq!(
            serde_json::to_value(&sandbox.settings).unwrap(),
            json!({
                "sso_start_url": "https://example.awsapps.com/start",
                "sso_region": "eu-west-1",
                "aws_access_key_id": "test",
            })
        );
    }

    #[test]
    fn load_profiles_missing_files_is_empty() {
        let profiles = load_profiles(
            PathBuf::from("/missing/config").as_path(),
            PathBuf::from("/missing/credentials").as_path(),
        )
        .unwrap();
        assert!(profiles.is_empty());
    }

    #[test]
    fn detects_sso_profiles() {
        assert!(is_sso_profile(&Profile {
            name: "sandbox".into(),
            settings: BTreeMap::from([("sso_session".into(), "shared".into())]),
        }));
        assert!(!is_sso_profile(&Profile {
            name: "prod".into(),
            settings: BTreeMap::from([("region".into(), "eu-west-1".into())]),
        }));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("sandbox"), "'sandbox'");
        assert_eq!(shell_quote("dev's account"), "'dev'\\''s account'");
    }
}
