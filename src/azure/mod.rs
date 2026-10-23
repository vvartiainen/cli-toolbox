use std::ffi::OsStr;
use std::io::Write;

use serde::Deserialize;

use crate::aws::shell_quote;
use crate::{Error, exec, select};

/// Environment variable read for the default tenant.
pub const TENANT_ENV: &str = "AZURE_TENANT_ID";

/// Variables exported for a selected subscription. `ARM_*` are read by
/// Terraform's azurerm provider.
const SUBSCRIPTION_VARS: [&str; 3] = [
    "AZURE_SUBSCRIPTION_ID",
    "ARM_SUBSCRIPTION_ID",
    "ARM_TENANT_ID",
];

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub id: String,
    pub name: String,
    pub tenant_id: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub user: Option<User>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct User {
    pub name: String,
}

impl Subscription {
    pub fn label(&self) -> String {
        let marker = if self.is_default { "* " } else { "  " };
        format!("{marker}{}  {}", self.name, self.id)
    }
}

/// Account tasks offered by the launcher.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountAction {
    Status,
    Login,
    Relogin,
    Logout,
}

impl AccountAction {
    pub const ALL: [Self; 4] = [Self::Status, Self::Login, Self::Relogin, Self::Logout];

    pub fn label(self) -> &'static str {
        match self {
            Self::Status => "Status",
            Self::Login => "Log in",
            Self::Relogin => "Log in again (after PIM activation)",
            Self::Logout => "Log out",
        }
    }
}

/// What an access token check is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope<'a> {
    Default,
    Tenant(&'a str),
    Subscription(&'a str),
}

/// The Azure CLI operations the helpers need.
pub trait Az {
    fn has_token(&self, scope: Scope) -> bool;
    fn login(&self, tenant: Option<&str>) -> Result<(), Error>;
    fn logout(&self) -> Result<(), Error>;
    fn subscriptions(&self) -> Result<Vec<Subscription>, Error>;
    fn set_subscription(&self, id: &str) -> Result<(), Error>;
    /// The default subscription, or `None` when not logged in.
    fn current(&self) -> Option<Subscription>;
    /// Expiry of the current access token, or `None` when there is none.
    fn token_expiry(&self) -> Option<String>;
}

/// The `az` executable.
pub struct AzCli;

impl AzCli {
    pub fn new() -> Result<Self, Error> {
        exec::require("az")?;
        Ok(Self)
    }
}

impl Az for AzCli {
    fn has_token(&self, scope: Scope) -> bool {
        let mut args = vec!["account", "get-access-token", "--output", "none"];
        match scope {
            Scope::Default => {}
            Scope::Tenant(tenant) => args.extend(["--tenant", tenant]),
            Scope::Subscription(id) => args.extend(["--subscription", id]),
        }
        exec::run_discard("az", &args).is_ok()
    }

    fn login(&self, tenant: Option<&str>) -> Result<(), Error> {
        let mut args = vec!["login", "--output", "none"];
        if let Some(tenant) = tenant {
            args.extend(["--tenant", tenant]);
        }
        // The new login experience asks for a subscription on stdin.
        exec::run_on_stderr_in(
            "az",
            &args,
            None,
            &[("AZURE_CORE_LOGIN_EXPERIENCE_V2", OsStr::new("off"))],
        )
    }

    fn logout(&self) -> Result<(), Error> {
        // Fails when nobody is logged in; clearing the cache is what matters.
        let _ = exec::run_discard("az", &["logout"]);
        exec::run_discard("az", &["account", "clear"])
    }

    fn subscriptions(&self) -> Result<Vec<Subscription>, Error> {
        let json = exec::captured_stdout("az", &["account", "list", "--output", "json"])?;
        parse_subscriptions(&json)
    }

    fn set_subscription(&self, id: &str) -> Result<(), Error> {
        exec::captured_stdout("az", &["account", "set", "--subscription", id]).map(|_| ())
    }

    fn current(&self) -> Option<Subscription> {
        let json = exec::captured_stdout("az", &["account", "show", "--output", "json"]).ok()?;
        serde_json::from_str(&json).ok()
    }

    fn token_expiry(&self) -> Option<String> {
        exec::captured_stdout(
            "az",
            &[
                "account",
                "get-access-token",
                "--query",
                "expiresOn",
                "--output",
                "tsv",
            ],
        )
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    }
}

pub fn parse_subscriptions(json: &str) -> Result<Vec<Subscription>, Error> {
    serde_json::from_str(json)
        .map_err(|err| Error::msg(format!("parse az account list output: {err}")))
}

/// The tenant from `--tenant`, else from `AZURE_TENANT_ID`.
pub fn resolve_tenant(flag: Option<String>, env: Option<String>) -> Option<String> {
    flag.or(env).filter(|tenant| !tenant.trim().is_empty())
}

pub fn tenant_from_env(flag: Option<String>) -> Option<String> {
    resolve_tenant(flag, std::env::var(TENANT_ENV).ok())
}

/// Log in when there is no valid session for `scope`.
pub fn ensure_login(
    az: &dyn Az,
    scope: Scope,
    tenant: Option<&str>,
    stderr: &mut dyn Write,
) -> Result<(), Error> {
    if az.has_token(scope) {
        return Ok(());
    }
    run_login(az, tenant, stderr)
}

fn run_login(az: &dyn Az, tenant: Option<&str>, stderr: &mut dyn Write) -> Result<(), Error> {
    match tenant {
        Some(tenant) => writeln!(stderr, "Running az login for tenant {tenant}...")?,
        None => writeln!(stderr, "Running az login...")?,
    }
    az.login(tenant)
        .map_err(|err| Error::msg(format!("az login failed: {err}")))
}

/// Log in (always with `force`, e.g. after activating a PIM role) and return
/// the status. The previously selected subscription stays selected.
pub fn login(
    az: &dyn Az,
    tenant: Option<&str>,
    force: bool,
    stderr: &mut dyn Write,
) -> Result<String, Error> {
    let scope = tenant.map_or(Scope::Default, Scope::Tenant);
    if force || !az.has_token(scope) {
        let previous = az.current();
        run_login(az, tenant, stderr)?;
        if let Some(previous) = previous
            && tenant.is_none_or(|tenant| tenant == previous.tenant_id)
        {
            // Ignore failures: the subscription may no longer be visible.
            let _ = az.set_subscription(&previous.id);
        }
    }
    Ok(status(az, None))
}

/// Log out and return a command that unsets the exported variables.
pub fn logout(az: &dyn Az, stderr: &mut dyn Write) -> Result<String, Error> {
    az.logout()?;
    writeln!(stderr, "Logged out of the Azure CLI.")?;
    Ok(format!("unset {}", SUBSCRIPTION_VARS.join(" ")))
}

/// Pick a subscription, logging in first when the CLI knows none, and return
/// its export command. `None` when the picker was cancelled.
pub fn select_subscription(
    az: &dyn Az,
    tenant: Option<&str>,
    stderr: &mut dyn Write,
) -> Result<Option<String>, Error> {
    let mut subscriptions = az.subscriptions()?;
    if subscriptions.is_empty() {
        run_login(az, tenant, stderr)?;
        subscriptions = az.subscriptions()?;
    }
    if subscriptions.is_empty() {
        return Err(Error::msg("no azure subscriptions are available"));
    }
    let labels: Vec<String> = subscriptions.iter().map(Subscription::label).collect();
    let Some(index) = select::pick("azure subscription> ", &labels, "~100%")? else {
        return Ok(None);
    };
    let selected = subscriptions
        .get(index)
        .ok_or_else(|| Error::msg("selected azure subscription was not found"))?;
    use_subscription(az, selected, stderr).map(Some)
}

/// Log in to the subscription's tenant if needed, make it the CLI default and
/// return its export command.
pub fn use_subscription(
    az: &dyn Az,
    subscription: &Subscription,
    stderr: &mut dyn Write,
) -> Result<String, Error> {
    ensure_login(
        az,
        Scope::Subscription(&subscription.id),
        Some(&subscription.tenant_id),
        stderr,
    )?;
    az.set_subscription(&subscription.id)?;
    Ok(export_command(subscription))
}

pub fn export_command(subscription: &Subscription) -> String {
    let id = shell_quote(&subscription.id);
    let tenant = shell_quote(&subscription.tenant_id);
    format!(
        "export {}={id} {}={id} {}={tenant}",
        SUBSCRIPTION_VARS[0], SUBSCRIPTION_VARS[1], SUBSCRIPTION_VARS[2]
    )
}

/// Account, subscription and token expiry. `shell_subscription` is the
/// exported `AZURE_SUBSCRIPTION_ID`, if any.
pub fn status(az: &dyn Az, shell_subscription: Option<&str>) -> String {
    let Some(current) = az.current() else {
        return "Not logged in to the Azure CLI.\nRun: cli-toolbox azure login".into();
    };
    let user = current.user.as_ref().map_or("unknown", |user| &user.name);
    let token = az
        .token_expiry()
        .unwrap_or_else(|| "none or expired (run: cli-toolbox azure login --force)".into());
    let mut lines = vec![
        format!("User:          {user}"),
        format!("Subscription:  {} ({})", current.name, current.id),
        format!("Tenant:        {}", current.tenant_id),
        format!("Token expires: {token}"),
    ];
    if let Some(shell) = shell_subscription.filter(|shell| !shell.is_empty()) {
        let note = if shell == current.id {
            ""
        } else {
            " (differs from the az default)"
        };
        lines.push(format!(
            "Shell:         AZURE_SUBSCRIPTION_ID={shell}{note}"
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeAz {
        token: bool,
        subscriptions: Vec<Subscription>,
        current: Option<Subscription>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeAz {
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Az for FakeAz {
        fn has_token(&self, scope: Scope) -> bool {
            self.calls.borrow_mut().push(format!("token {scope:?}"));
            self.token
        }
        fn login(&self, tenant: Option<&str>) -> Result<(), Error> {
            self.calls.borrow_mut().push(format!("login {tenant:?}"));
            Ok(())
        }
        fn logout(&self) -> Result<(), Error> {
            self.calls.borrow_mut().push("logout".into());
            Ok(())
        }
        fn subscriptions(&self) -> Result<Vec<Subscription>, Error> {
            Ok(self.subscriptions.clone())
        }
        fn set_subscription(&self, id: &str) -> Result<(), Error> {
            self.calls.borrow_mut().push(format!("set {id}"));
            Ok(())
        }
        fn current(&self) -> Option<Subscription> {
            self.current.clone()
        }
        fn token_expiry(&self) -> Option<String> {
            self.token.then(|| "2026-10-23 11:00:00".into())
        }
    }

    fn subscription(id: &str, tenant: &str) -> Subscription {
        Subscription {
            id: id.into(),
            name: format!("{id}-name"),
            tenant_id: tenant.into(),
            is_default: false,
            user: Some(User {
                name: "me@example.com".into(),
            }),
        }
    }

    #[test]
    fn parses_az_account_list() {
        let json = r#"[{"cloudName":"AzureCloud","id":"sub-1","isDefault":true,
            "name":"dev","state":"Enabled","tenantId":"tenant-1",
            "user":{"name":"me@example.com","type":"user"}}]"#;
        let subscriptions = parse_subscriptions(json).unwrap();
        assert_eq!(subscriptions.len(), 1);
        assert_eq!(subscriptions[0].tenant_id, "tenant-1");
        assert_eq!(subscriptions[0].label(), "* dev  sub-1");
    }

    #[test]
    fn tenant_flag_wins_over_env_and_blank_is_none() {
        assert_eq!(
            resolve_tenant(Some("flag".into()), Some("env".into())).as_deref(),
            Some("flag")
        );
        assert_eq!(
            resolve_tenant(None, Some("env".into())).as_deref(),
            Some("env")
        );
        assert_eq!(resolve_tenant(None, Some(" ".into())), None);
        assert_eq!(resolve_tenant(None, None), None);
    }

    #[test]
    fn export_sets_cli_and_terraform_variables() {
        assert_eq!(
            export_command(&subscription("sub-1", "tenant-1")),
            "export AZURE_SUBSCRIPTION_ID='sub-1' ARM_SUBSCRIPTION_ID='sub-1' ARM_TENANT_ID='tenant-1'"
        );
    }

    #[test]
    fn using_a_subscription_with_a_token_does_not_log_in() {
        let az = FakeAz {
            token: true,
            ..FakeAz::default()
        };
        let mut stderr = Vec::new();
        use_subscription(&az, &subscription("sub-1", "tenant-1"), &mut stderr).unwrap();
        assert_eq!(az.calls(), ["token Subscription(\"sub-1\")", "set sub-1"]);
        assert!(stderr.is_empty());
    }

    #[test]
    fn using_a_subscription_without_a_token_logs_in_to_its_tenant() {
        let az = FakeAz::default();
        let mut stderr = Vec::new();
        use_subscription(&az, &subscription("sub-1", "tenant-1"), &mut stderr).unwrap();
        assert_eq!(
            az.calls(),
            [
                "token Subscription(\"sub-1\")",
                "login Some(\"tenant-1\")",
                "set sub-1"
            ]
        );
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            "Running az login for tenant tenant-1...\n"
        );
    }

    #[test]
    fn forced_login_keeps_the_previous_subscription() {
        let az = FakeAz {
            token: true,
            current: Some(subscription("sub-1", "tenant-1")),
            ..FakeAz::default()
        };
        login(&az, Some("tenant-1"), true, &mut Vec::new()).unwrap();
        assert_eq!(az.calls(), ["login Some(\"tenant-1\")", "set sub-1"]);
    }

    #[test]
    fn login_with_a_valid_session_only_reports_status() {
        let az = FakeAz {
            token: true,
            current: Some(subscription("sub-1", "tenant-1")),
            ..FakeAz::default()
        };
        let status = login(&az, None, false, &mut Vec::new()).unwrap();
        assert_eq!(az.calls(), ["token Default"]);
        assert!(status.contains("User:          me@example.com"), "{status}");
    }

    #[test]
    fn logout_returns_an_unset_command() {
        let az = FakeAz::default();
        let unset = logout(&az, &mut Vec::new()).unwrap();
        assert_eq!(az.calls(), ["logout"]);
        assert_eq!(
            unset,
            "unset AZURE_SUBSCRIPTION_ID ARM_SUBSCRIPTION_ID ARM_TENANT_ID"
        );
    }

    #[test]
    fn status_reports_missing_login_and_shell_mismatch() {
        assert!(status(&FakeAz::default(), None).starts_with("Not logged in"));
        let az = FakeAz {
            token: false,
            current: Some(subscription("sub-1", "tenant-1")),
            ..FakeAz::default()
        };
        let text = status(&az, Some("sub-2"));
        assert!(text.contains("Token expires: none or expired"), "{text}");
        assert!(
            text.contains("AZURE_SUBSCRIPTION_ID=sub-2 (differs from the az default)"),
            "{text}"
        );
    }
}
