mod config;

use std::io::Write;
use std::path::Path;

pub use config::{Profile, load_profiles};

use crate::{Error, exec, select};

pub fn select_and_export_profile(
    home: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    let config_path = home.join(".aws").join("config");
    let credentials_path = home.join(".aws").join("credentials");
    let profiles = load_profiles(&config_path, &credentials_path)?;
    if profiles.is_empty() {
        return Err(Error::msg(format!(
            "no aws profiles found in \"{}\" or \"{}\"",
            config_path.display(),
            credentials_path.display()
        )));
    }

    let Some(selected) = select_profile(&profiles)? else {
        return Ok(());
    };

    ensure_sso_login(selected, stderr)?;
    writeln!(
        stdout,
        "export AWS_PROFILE={}",
        config::shell_quote(&selected.name)
    )?;
    Ok(())
}

fn select_profile(profiles: &[Profile]) -> Result<Option<&Profile>, Error> {
    let lines: Vec<String> = profiles
        .iter()
        .map(|profile| profile.name.clone())
        .collect();
    let Some(index) = select::pick("aws profile> ", &lines, "~100%")? else {
        return Ok(None);
    };
    profiles
        .get(index)
        .ok_or_else(|| Error::msg("selected aws profile was not found"))
        .map(Some)
}

fn ensure_sso_login(profile: &Profile, stderr: &mut dyn Write) -> Result<(), Error> {
    if !config::is_sso_profile(profile) {
        return Ok(());
    }

    exec::require("aws")?;
    if has_valid_session(&profile.name) {
        return Ok(());
    }

    writeln!(
        stderr,
        "Running aws sso login for profile \"{}\"...",
        profile.name
    )?;
    exec::run_on_stderr("aws", &["sso", "login", "--profile", &profile.name]).map_err(|err| {
        Error::msg(format!(
            "aws sso login failed for profile \"{}\": {err}",
            profile.name
        ))
    })
}

fn has_valid_session(profile_name: &str) -> bool {
    exec::run_discard(
        "aws",
        &["sts", "get-caller-identity", "--profile", profile_name],
    )
    .is_ok()
}
