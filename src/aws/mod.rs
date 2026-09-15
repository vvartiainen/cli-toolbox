mod config;

use std::io::Write;
use std::path::Path;

pub(crate) use config::shell_quote;
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

    let export = export_profile(selected, stderr)?;
    writeln!(stdout, "{export}")?;
    Ok(())
}

pub(crate) fn export_profile(profile: &Profile, stderr: &mut dyn Write) -> Result<String, Error> {
    ensure_sso_login(profile, stderr)?;
    Ok(format!("export AWS_PROFILE={}", shell_quote(&profile.name)))
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
    ensure_sso_login_with(profile, stderr, has_valid_session, |profile_name| {
        exec::run_on_stderr("aws", &["sso", "login", "--profile", profile_name])
    })
}

fn ensure_sso_login_with(
    profile: &Profile,
    stderr: &mut dyn Write,
    has_valid_session: impl Fn(&str) -> bool,
    run_login: impl Fn(&str) -> Result<(), Error>,
) -> Result<(), Error> {
    if has_valid_session(&profile.name) {
        return Ok(());
    }

    writeln!(
        stderr,
        "Running aws sso login for profile \"{}\"...",
        profile.name
    )?;
    run_login(&profile.name).map_err(|err| {
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

#[cfg(test)]
mod tests {
    use super::{Profile, ensure_sso_login_with, export_profile};
    use std::collections::BTreeMap;

    #[test]
    fn export_profile_formats_a_shell_command_without_a_newline() {
        let profile = Profile {
            name: "dev's account".into(),
            settings: BTreeMap::new(),
        };
        let mut stderr = Vec::new();

        let export = export_profile(&profile, &mut stderr).unwrap();

        assert_eq!(export, "export AWS_PROFILE='dev'\\''s account'");
        assert!(stderr.is_empty());
    }

    #[test]
    fn valid_sso_session_does_not_log_in_again() {
        let profile = Profile {
            name: "sandbox".into(),
            settings: BTreeMap::new(),
        };
        let mut stderr = Vec::new();
        ensure_sso_login_with(
            &profile,
            &mut stderr,
            |name| {
                assert_eq!(name, "sandbox");
                true
            },
            |_| panic!("valid sessions must not trigger login"),
        )
        .unwrap();
        assert!(stderr.is_empty());
    }

    #[test]
    fn expired_sso_session_logs_in_with_the_selected_profile() {
        let profile = Profile {
            name: "sandbox".into(),
            settings: BTreeMap::new(),
        };
        let logged_in = std::cell::Cell::new(false);
        ensure_sso_login_with(
            &profile,
            &mut Vec::new(),
            |_| false,
            |name| {
                assert_eq!(name, "sandbox");
                logged_in.set(true);
                Ok(())
            },
        )
        .unwrap();
        assert!(logged_in.get());
    }

    #[test]
    fn mocked_sso_failure_is_reported_without_an_export() {
        let profile = Profile {
            name: "sandbox".into(),
            settings: BTreeMap::from([(String::from("sso_start_url"), String::from("url"))]),
        };
        let mut stderr = Vec::new();

        let result = ensure_sso_login_with(
            &profile,
            &mut stderr,
            |_| false,
            |_| Err(crate::Error::msg("mock login failed")),
        );

        let error = result.unwrap_err();
        assert_eq!(
            error.to_string(),
            "aws sso login failed for profile \"sandbox\": mock login failed"
        );
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "Running aws sso login for profile \"sandbox\"...\n"
        );
    }
}
