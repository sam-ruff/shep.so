//! Live IMAP launch: one real account from environment variables with a
//! memory-only keychain, for the gated e2e mailbox scenarios. Everything
//! else runs the production path; no fixture mail or fictional provider.
use crate::{credentials, model::*, store::Store};
use anyhow::Context;
use secrecy::{ExposeSecret, SecretString};
use std::collections::BTreeMap;

pub const ACCOUNT_ID: &str = "live-e2e";
const VARIABLES: [&str; 6] = [
    "SHEP_LIVE_IMAP_HOST",
    "SHEP_LIVE_IMAP_PORT",
    "SHEP_LIVE_IMAP_USER",
    "SHEP_LIVE_IMAP_PASSWORD",
    "SHEP_LIVE_SMTP_HOST",
    "SHEP_LIVE_SMTP_PORT",
];

pub fn active() -> bool {
    std::env::args().any(|arg| arg == "--live-imap")
}

/// The data root of a live launch: a `live-data` directory beside the owned
/// `--test-state` file, so the personal data root is never opened.
pub fn data_root() -> anyhow::Result<Option<std::path::PathBuf>> {
    if !active() {
        return Ok(None);
    }
    let arguments: Vec<_> = std::env::args_os().collect();
    data_root_from(&arguments).map(Some)
}

fn data_root_from(arguments: &[std::ffi::OsString]) -> anyhow::Result<std::path::PathBuf> {
    let state = arguments
        .windows(2)
        .find(|pair| pair[0] == "--test-state")
        .map(|pair| std::path::Path::new(&pair[1]))
        .context("A live launch requires --test-state in an owned artifact directory")?;
    let directory = state
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("The live state file needs a parent directory")?;
    Ok(directory.join("live-data"))
}

/// The password is a `SecretString`, so debug output redacts it.
#[derive(Debug)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: SecretString,
    pub smtp_host: String,
    pub smtp_port: u16,
}

/// Reads the live account from the launch environment when `--live-imap` is set.
pub fn settings() -> anyhow::Result<Option<Settings>> {
    if !active() {
        return Ok(None);
    }
    settings_from(|name| std::env::var(name).ok()).map(Some)
}

pub fn settings_from(variable: impl Fn(&str) -> Option<String>) -> anyhow::Result<Settings> {
    let mut values = BTreeMap::new();
    for name in VARIABLES {
        let value = variable(name)
            .filter(|value| !value.trim().is_empty())
            .with_context(|| format!("The live launch needs {name}"))?;
        values.insert(name, value);
    }
    let port = |name: &str| -> anyhow::Result<u16> {
        values[name]
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .with_context(|| format!("{name} must be a port between 1 and 65535"))
    };
    Ok(Settings {
        host: values["SHEP_LIVE_IMAP_HOST"].trim().to_owned(),
        port: port("SHEP_LIVE_IMAP_PORT")?,
        user: values["SHEP_LIVE_IMAP_USER"].trim().to_owned(),
        password: SecretString::from(values["SHEP_LIVE_IMAP_PASSWORD"].clone()),
        smtp_host: values["SHEP_LIVE_SMTP_HOST"].trim().to_owned(),
        smtp_port: port("SHEP_LIVE_SMTP_PORT")?,
    })
}

impl Settings {
    pub fn account(&self) -> Account {
        Account {
            id: ACCOUNT_ID.into(),
            name: "Live e2e".into(),
            email: self.user.clone(),
            protocol: Protocol::Imap,
            host: self.host.clone(),
            port: self.port,
            username: self.user.clone(),
            smtp_host: self.smtp_host.clone(),
            smtp_port: self.smtp_port,
            incoming_security: if self.port == 143 {
                ConnectionSecurity::StartTls
            } else {
                ConnectionSecurity::Tls
            },
            incoming_auth: IncomingAuth::Password,
            smtp_security: None,
            smtp_auth: SmtpAuth::Automatic,
            smtp_username: String::new(),
            smtp_separate_password: false,
            sent_copy: Default::default(),
            sent_folder: String::new(),
        }
    }

    /// A keychain that lives only in this process; the OS keychain is never
    /// read or written by a live launch.
    pub async fn credentials(
        &self,
        scope: credentials::Scope,
    ) -> anyhow::Result<credentials::Credentials> {
        let credentials = credentials::Credentials::with_backend(scope, MemoryKeychain::default());
        credentials
            .write(
                ACCOUNT_ID,
                SecretString::from(self.password.expose_secret().to_owned()),
            )
            .await?;
        Ok(credentials)
    }

    pub async fn seed(&self, store: &Store) -> anyhow::Result<()> {
        let account = self.account();
        account.validate()?;
        store.save_account(account).await
    }
}

#[derive(Default)]
struct MemoryKeychain(BTreeMap<String, SecretString>);
impl credentials::Backend for MemoryKeychain {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(self.0.get(key).cloned())
    }
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        self.0.insert(key.into(), value);
        Ok(())
    }
    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.0.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn variables<'a>(overrides: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            if let Some((_, value)) = overrides.iter().find(|(key, _)| *key == name) {
                return (!value.is_empty()).then(|| (*value).to_owned());
            }
            Some(
                match name {
                    "SHEP_LIVE_IMAP_HOST" => "imap.example",
                    "SHEP_LIVE_IMAP_PORT" => "993",
                    "SHEP_LIVE_IMAP_USER" => "e2e@example",
                    "SHEP_LIVE_IMAP_PASSWORD" => "secret",
                    "SHEP_LIVE_SMTP_HOST" => "smtp.example",
                    "SHEP_LIVE_SMTP_PORT" => "587",
                    _ => return None,
                }
                .to_owned(),
            )
        }
    }

    #[test]
    fn settings_build_a_tls_imap_account_with_submission_smtp() {
        let settings = settings_from(variables(&[])).unwrap();
        let account = settings.account();
        assert_eq!(account.id, ACCOUNT_ID);
        assert_eq!(account.protocol, Protocol::Imap);
        assert_eq!((account.host.as_str(), account.port), ("imap.example", 993));
        assert_eq!(account.incoming_security, ConnectionSecurity::Tls);
        assert_eq!(account.smtp_security(), ConnectionSecurity::StartTls);
        assert_eq!(account.username, "e2e@example");
        assert!(account.validate().is_ok());
    }

    #[test]
    fn every_variable_is_required_and_ports_are_checked() {
        for name in VARIABLES {
            let error = settings_from(variables(&[(name, "")])).unwrap_err();
            assert!(error.to_string().contains(name), "{error}");
        }
        let error = settings_from(variables(&[("SHEP_LIVE_IMAP_PORT", "imaps")])).unwrap_err();
        assert!(error.to_string().contains("SHEP_LIVE_IMAP_PORT"));
        assert!(
            settings_from(variables(&[("SHEP_LIVE_SMTP_PORT", "0")]))
                .unwrap_err()
                .to_string()
                .contains("SHEP_LIVE_SMTP_PORT")
        );
    }

    #[test]
    fn the_data_root_sits_beside_the_owned_state_file() {
        let arguments: Vec<std::ffi::OsString> = [
            "shep",
            "--live-imap",
            "--test-state",
            "/run/owned/state.json",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        assert_eq!(
            data_root_from(&arguments).unwrap(),
            std::path::Path::new("/run/owned/live-data")
        );
        let bare: Vec<std::ffi::OsString> = ["shep", "--live-imap"]
            .into_iter()
            .map(Into::into)
            .collect();
        assert!(data_root_from(&bare).is_err());
    }

    #[tokio::test]
    async fn the_memory_keychain_holds_only_the_live_password() {
        let settings = settings_from(variables(&[])).unwrap();
        let credentials = settings
            .credentials(credentials::Scope::Legacy)
            .await
            .unwrap();
        assert_eq!(
            credentials.read(ACCOUNT_ID).await.unwrap().expose_secret(),
            "secret"
        );
        assert!(credentials.read_optional("other").await.unwrap().is_none());
        let store = Store::memory().unwrap();
        settings.seed(&store).await.unwrap();
        let accounts = store.get::<Vec<Account>>("accounts").await.unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].email, "e2e@example");
    }
}
