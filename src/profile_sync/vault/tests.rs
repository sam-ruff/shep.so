use super::*;
use crate::{
    credentials::{Backend, Scope as CredentialScope},
    model::{ConnectionSecurity, IncomingAuth, Protocol},
    profile_sync::enrollment::{Changes, Options, Origin, Selection},
};
use secrecy::ExposeSecret;
use shep_profile_core::vault::{self as codec, Key, Value, Vault};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(crate) const INCOMING: &str = "fixture vault incoming ✓";
pub(crate) const SMTP: &str = "fixture vault smtp";

/// In-memory app-data store. Every call yields so concurrent passes interleave.
#[derive(Default)]
pub(crate) struct Cloud {
    files: Mutex<BTreeMap<String, (RemoteFile, Vec<u8>)>>,
    next: AtomicUsize,
    pub(crate) creates: AtomicUsize,
    pub(crate) deletes: AtomicUsize,
    key_creates: AtomicUsize,
    fail_create: Mutex<bool>,
    /// Holds the first listings until two passes have both listed.
    race: Option<tokio::sync::Barrier>,
    lists: AtomicUsize,
}
impl Cloud {
    fn racing() -> Self {
        Self {
            race: Some(tokio::sync::Barrier::new(2)),
            ..Default::default()
        }
    }
    pub(crate) fn files(&self) -> Vec<(RemoteFile, Vec<u8>)> {
        self.files.lock().unwrap().values().cloned().collect()
    }
    fn of(&self, keys: bool) -> Vec<(RemoteFile, Vec<u8>)> {
        self.files()
            .into_iter()
            .filter(|(f, _)| matches!(f.kind, Kind::Key { .. }) == keys)
            .collect()
    }
    fn replace(&self, id: &str, bytes: Vec<u8>) {
        let mut files = self.files.lock().unwrap();
        let (file, old) = files.get_mut(id).unwrap();
        file.size = bytes.len() as u64;
        file.sha256 = crate::profile_sync::digest(&bytes);
        *old = bytes;
    }
}
#[async_trait]
impl Remote for Cloud {
    async fn list(&self, _: Scope) -> anyhow::Result<Vec<RemoteFile>> {
        tokio::task::yield_now().await;
        if let Some(race) = &self.race
            && self.lists.fetch_add(1, Ordering::SeqCst) < 2
        {
            race.wait().await;
        }
        Ok(self.files().into_iter().map(|(f, _)| f).collect())
    }
    async fn download(&self, _: Scope, file: &RemoteFile) -> anyhow::Result<Vec<u8>> {
        tokio::task::yield_now().await;
        self.files
            .lock()
            .unwrap()
            .get(&file.id)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| anyhow::anyhow!("gone"))
    }
    async fn create(&self, _: Scope, file: NewFile) -> anyhow::Result<RemoteFile> {
        tokio::task::yield_now().await;
        anyhow::ensure!(
            !*self.fail_create.lock().unwrap(),
            "Fixture Drive is offline."
        );
        self.creates.fetch_add(1, Ordering::SeqCst);
        if matches!(file.kind, Kind::Key { .. }) {
            self.key_creates.fetch_add(1, Ordering::SeqCst);
        }
        let remote = RemoteFile {
            id: format!("credential-{}", self.next.fetch_add(1, Ordering::SeqCst)),
            key: file.key,
            kind: file.kind,
            size: file.bytes.len() as u64,
            sha256: crate::profile_sync::digest(&file.bytes),
        };
        self.files
            .lock()
            .unwrap()
            .insert(remote.id.clone(), (remote.clone(), file.bytes));
        Ok(remote)
    }
    async fn delete(&self, file: &RemoteFile) -> anyhow::Result<()> {
        tokio::task::yield_now().await;
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.files.lock().unwrap().remove(&file.id);
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(crate) struct Keychain(pub(crate) Arc<Mutex<BTreeMap<String, String>>>);
impl Backend for Keychain {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .map(SecretString::from))
    }
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(key.into(), value.expose_secret().into());
        Ok(())
    }
    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

/// Accepts listed passwords and records every login attempt.
#[derive(Default)]
pub(crate) struct Servers {
    pub(crate) accept: Vec<String>,
    pub(crate) attempts: Mutex<Vec<(String, ConnectionTarget)>>,
}
#[async_trait]
impl Tester for Servers {
    async fn test(
        &self,
        account: &Account,
        target: ConnectionTarget,
        secret: &SecretString,
    ) -> anyhow::Result<()> {
        self.attempts
            .lock()
            .unwrap()
            .push((account.host.clone(), target));
        anyhow::ensure!(
            self.accept.iter().any(|s| s == secret.expose_secret()),
            "Authentication failed."
        );
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct History(pub(crate) BTreeSet<Uuid>);
#[async_trait]
impl Tombstones for History {
    async fn removed(&self, account: Uuid) -> anyhow::Result<bool> {
        Ok(self.0.contains(&account))
    }
}

pub(crate) fn binding() -> history::Binding {
    history::Binding {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture-user".into(),
        profile: Uuid::from_u128(0x10),
        generation: Uuid::from_u128(0x20),
    }
}
pub(crate) fn account(id: &str, host: &str, separate: bool) -> Account {
    Account {
        id: id.into(),
        name: "Studio".into(),
        email: "alex@studio.test".into(),
        protocol: Protocol::Imap,
        host: host.into(),
        port: 993,
        username: "alex@studio.test".into(),
        smtp_host: "smtp.studio.test".into(),
        smtp_port: 465,
        incoming_security: ConnectionSecurity::Tls,
        incoming_auth: IncomingAuth::Password,
        smtp_security: Some(ConnectionSecurity::Tls),
        smtp_auth: SmtpAuth::Login,
        smtp_username: "alex@studio.test".into(),
        smtp_separate_password: separate,
        sent_copy: Default::default(),
        sent_folder: String::new(),
    }
}

pub(crate) struct Device {
    pub(crate) store: Store,
    pub(crate) keychain: Keychain,
    pub(crate) credentials: Credentials,
}
impl Device {
    pub(crate) fn secret(&self, key: &str) -> Option<String> {
        self.keychain.0.lock().unwrap().get(key).cloned()
    }
    pub(crate) async fn local(&self) -> Local {
        self.store.get(STORAGE_KEY).await.unwrap()
    }
    pub(crate) async fn reconnecting(&self) -> BTreeSet<String> {
        self.store
            .get(crate::profile_sync::join::RECONNECT_KEY)
            .await
            .unwrap()
    }
}

/// An enrolled, ready device whose native accounts map to shared UUIDs.
pub(crate) async fn device(
    path: Option<&Path>,
    accounts: &[(Account, Uuid)],
    reconnect: bool,
    secrets: &[(&str, &str)],
) -> Device {
    let store = match path {
        Some(path) => Store::open(path.join("cache.sqlite")).unwrap(),
        None => Store::memory().unwrap(),
    };
    store
        .update_preferences(|p| {
            p.google_client_id = "fixture-client".into();
            p.google_connection_id = "drive:fixture-user".into();
            p.google_grant = crate::model::GoogleGrant {
                id: "fixture-grant".into(),
                client_id: "fixture-client".into(),
                access: crate::model::GoogleAccess {
                    known: true,
                    drive: true,
                    calendar_read: false,
                    calendar_write: false,
                },
            };
        })
        .await
        .unwrap();
    for (account, _) in accounts {
        store.save_account(account.clone()).await.unwrap();
    }
    if reconnect {
        let pending: BTreeSet<String> = accounts.iter().map(|(a, _)| a.id.clone()).collect();
        store
            .put(crate::profile_sync::join::RECONNECT_KEY, pending)
            .await
            .unwrap();
    }
    let pending = store
        .begin_profile_enrollment(
            store.profile_enrollment().await.unwrap(),
            Selection {
                binding: binding(),
                name: "Personal".into(),
                origin: Origin::Join,
                ready: false,
            },
            Options {
                enabled: true,
                passwords: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ready = store.profile_sync_succeeded(pending, 0).await.unwrap();
    store
        .initialize_profile_replication(
            ready,
            0,
            accounts.iter().map(|(a, s)| (a.id.clone(), *s)).collect(),
            vec![],
        )
        .await
        .unwrap();
    let keychain = Keychain::default();
    for (key, value) in secrets {
        keychain
            .0
            .lock()
            .unwrap()
            .insert((*key).into(), (*value).into());
    }
    let credentials = Credentials::with_backend(CredentialScope::Legacy, keychain.clone());
    Device {
        store,
        keychain,
        credentials,
    }
}

/// The engine's sequence without its locks: reconcile, then stage, test and
/// activate each import, keeping the active pair when a test fails.
pub(crate) async fn pass(
    device: &Device,
    cloud: &dyn Remote,
    servers: &Servers,
    history: &History,
    retry: bool,
) -> anyhow::Result<Report> {
    let control = Control::default();
    let ctx = Context {
        store: &device.store,
        credentials: &device.credentials,
        remote: cloud,
        tester: servers,
        history,
        control: &control,
        retry_failed: retry,
    };
    let outcome = reconcile(&ctx).await?;
    let mut report = outcome.report;
    for import in outcome.imports {
        stage(&ctx, &import).await?;
        match test(&ctx, &import).await {
            Ok(()) => {
                activate(&ctx, &import).await?;
                report.imported += 1;
            }
            Err(_) => {
                report.failed += 1;
                record_failure(&ctx, &import).await?;
            }
        }
        unstage(&ctx, &import).await?;
    }
    Ok(report)
}

fn shared(n: u128) -> Uuid {
    Uuid::from_u128(0x5000 + n)
}
fn accepting(secrets: &[&str]) -> Servers {
    Servers {
        accept: secrets.iter().map(|s| (*s).to_owned()).collect(),
        ..Default::default()
    }
}
/// Decode the single remaining vault with its key file.
fn opened(cloud: &Cloud) -> (Key, Vault) {
    let keys = cloud.of(true);
    let vaults = cloud.of(false);
    assert_eq!((keys.len(), vaults.len()), (1, 1), "one key and one vault");
    let scope = binding();
    let key = Key::decode(&keys[0].1, scope.profile, scope.generation).unwrap();
    let vault = Vault::decode(&vaults[0].1, scope.profile, scope.generation).unwrap();
    assert_eq!(vault.key, key.id());
    (key, vault)
}
fn secret_in(key: &Key, vault: &Vault, account: Uuid, field: Field) -> Option<String> {
    let entry = vault
        .entries
        .iter()
        .find(|e| e.account == account && e.field == field)?;
    let Value::Sealed { sealed, .. } = &entry.value else {
        return None;
    };
    Some(String::from_utf8(codec::open(key, &entry.slot()?, sealed).unwrap().to_vec()).unwrap())
}

#[tokio::test]
async fn profile_vault_publishes_then_a_reconnecting_device_imports_after_testing() {
    let cloud = Cloud::default();
    let studio = shared(1);
    let a = device(
        None,
        &[(
            account(&studio.to_string(), "imap.studio.test", true),
            studio,
        )],
        false,
        &[
            (&studio.to_string(), INCOMING),
            (&format!("{studio}:smtp"), SMTP),
        ],
    )
    .await;
    let report = pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    assert_eq!(report.published, 2);
    let (key, vault) = opened(&cloud);
    assert_eq!(key.sequence(), 1);
    assert_eq!(
        secret_in(&key, &vault, studio, Field::Incoming).as_deref(),
        Some(INCOMING)
    );
    assert_eq!(
        secret_in(&key, &vault, studio, Field::Smtp).as_deref(),
        Some(SMTP)
    );
    for (_, bytes) in cloud.files() {
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(INCOMING) && !text.contains(SMTP));
    }
    assert_eq!(a.local().await.synced(studio, Field::Incoming).revision, 1);

    // Device B imported the account through the shared profile and must
    // reconnect it. The vault pair is tested, then activated.
    let b = device(
        None,
        &[(account("b-native", "imap.studio.test", true), studio)],
        true,
        &[],
    )
    .await;
    let servers = accepting(&[INCOMING, SMTP]);
    let report = pass(&b, &cloud, &servers, &History::default(), false)
        .await
        .unwrap();
    assert_eq!(
        (report.imported, report.failed, report.published),
        (1, 0, 0)
    );
    assert_eq!(b.secret("b-native").as_deref(), Some(INCOMING));
    assert_eq!(b.secret("b-native:smtp").as_deref(), Some(SMTP));
    assert!(
        b.secret("b-native:vault-incoming").is_none() && b.secret("b-native:vault-smtp").is_none()
    );
    assert!(b.reconnecting().await.is_empty());
    assert!(
        b.store
            .require_account_reconnected("b-native".into())
            .await
            .is_ok()
    );
    assert_eq!(
        *servers.attempts.lock().unwrap(),
        [
            ("imap.studio.test".to_owned(), ConnectionTarget::Incoming),
            ("imap.studio.test".to_owned(), ConnectionTarget::Smtp)
        ]
    );
    let local = b.local().await;
    assert_eq!(local.synced(studio, Field::Smtp).revision, 1);
    assert!(local.staged.is_empty());

    // Settled devices neither write nor test again.
    let creates = cloud.creates.load(Ordering::SeqCst);
    for device in [&a, &b] {
        let report = pass(device, &cloud, &servers, &History::default(), false)
            .await
            .unwrap();
        assert_eq!(report, Report::default());
    }
    assert_eq!(cloud.creates.load(Ordering::SeqCst), creates);
    assert_eq!(servers.attempts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn profile_vault_failed_test_keeps_the_active_pair_until_an_explicit_retry() {
    let cloud = Cloud::default();
    let studio = shared(1);
    let a = device(
        None,
        &[(
            account(&studio.to_string(), "imap.studio.test", false),
            studio,
        )],
        false,
        &[(&studio.to_string(), "old password")],
    )
    .await;
    let b = device(
        None,
        &[(account("b-native", "imap.studio.test", false), studio)],
        false,
        &[("b-native", "old password")],
    )
    .await;
    for device in [&a, &b] {
        pass(
            device,
            &cloud,
            &Servers::default(),
            &History::default(),
            false,
        )
        .await
        .unwrap();
    }
    // A changes its password; B's server still rejects the new one.
    a.keychain
        .0
        .lock()
        .unwrap()
        .insert(studio.to_string(), INCOMING.into());
    let report = pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    assert_eq!(report.published, 1);
    let rejecting = Servers::default();
    let report = pass(&b, &cloud, &rejecting, &History::default(), false)
        .await
        .unwrap();
    assert_eq!((report.imported, report.failed), (0, 1));
    assert_eq!(b.secret("b-native").as_deref(), Some("old password"));
    assert!(b.secret("b-native:vault-incoming").is_none());
    assert_eq!(b.local().await.synced(studio, Field::Incoming).failed, 2);
    // The failed revision is neither retried automatically nor overwritten,
    // and later passes keep reporting it as held for an explicit retry.
    let report = pass(&b, &cloud, &rejecting, &History::default(), false)
        .await
        .unwrap();
    assert_eq!(
        report,
        Report {
            held: 1,
            ..Default::default()
        }
    );
    assert_eq!(rejecting.attempts.lock().unwrap().len(), 1);
    let (key, vault) = opened(&cloud);
    assert_eq!(
        secret_in(&key, &vault, studio, Field::Incoming).as_deref(),
        Some(INCOMING)
    );
    // An explicit retry tests it again and activates it once it connects.
    let report = pass(
        &b,
        &cloud,
        &accepting(&[INCOMING]),
        &History::default(),
        true,
    )
    .await
    .unwrap();
    assert_eq!(report.imported, 1);
    assert_eq!(b.secret("b-native").as_deref(), Some(INCOMING));
}

#[tokio::test]
async fn profile_vault_removal_rotates_the_key_and_keeps_other_accounts() {
    let cloud = Cloud::default();
    let (studio, home) = (shared(1), shared(2));
    let accounts = [
        (
            account(&studio.to_string(), "imap.studio.test", false),
            studio,
        ),
        (account(&home.to_string(), "imap.home.test", false), home),
    ];
    let a = device(
        None,
        &accounts,
        false,
        &[
            (&studio.to_string(), INCOMING),
            (&home.to_string(), "home password"),
        ],
    )
    .await;
    pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    let (first, _) = opened(&cloud);
    // Removing Home on this device removes the entry it published.
    let mut remaining: Vec<Account> = a.store.get("accounts").await.unwrap();
    remaining.retain(|x| x.id != home.to_string());
    a.store.put("accounts", remaining).await.unwrap();
    let report = pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    assert!(report.rotated);
    assert_eq!(report.removed, 1);
    let (second, vault) = opened(&cloud);
    assert_ne!(second.id(), first.id());
    assert_eq!(second.sequence(), 2);
    assert_eq!(
        secret_in(&second, &vault, studio, Field::Incoming).as_deref(),
        Some(INCOMING)
    );
    let removed = vault.entries.iter().find(|e| e.account == home).unwrap();
    assert_eq!(
        (removed.value.clone(), removed.revision),
        (Value::Removed, 2)
    );
    assert!(
        a.local()
            .await
            .slots
            .keys()
            .all(|k| !k.starts_with(&home.to_string()))
    );

    // A shared-history removal makes any device remove the entry, even one
    // another device published, and rotate again.
    let b = device(
        None,
        &[(account("b-native", "imap.studio.test", false), studio)],
        false,
        &[("b-native", INCOMING)],
    )
    .await;
    let report = pass(
        &b,
        &cloud,
        &Servers::default(),
        &History(BTreeSet::from([studio])),
        false,
    )
    .await
    .unwrap();
    assert!(report.rotated);
    // Nothing sealed remains, so every credential file is gone.
    assert!(cloud.files().is_empty());
    assert_eq!(b.secret("b-native").as_deref(), Some(INCOMING));
}

#[tokio::test]
async fn profile_vault_toggle_off_removes_this_devices_entries_even_while_paused() {
    let cloud = Cloud::default();
    let studio = shared(1);
    let a = device(
        None,
        &[(
            account(&studio.to_string(), "imap.studio.test", true),
            studio,
        )],
        false,
        &[
            (&studio.to_string(), INCOMING),
            (&format!("{studio}:smtp"), SMTP),
        ],
    )
    .await;
    pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    assert_eq!(cloud.files().len(), 2);
    // Pausing sync first must not stop the removal the toggle asks for.
    for changes in [
        Changes {
            enabled: Some(false),
            ..Default::default()
        },
        Changes {
            passwords: Some(false),
            ..Default::default()
        },
    ] {
        a.store.change_profile_sync_options(changes).await.unwrap();
    }
    assert!(a.local().await.withdraw);
    let report = pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    assert!(report.withdrawn);
    assert_eq!(report.removed, 2);
    assert!(cloud.files().is_empty());
    let local = a.local().await;
    assert!(!local.withdraw && local.slots.is_empty());
    // The keychain keeps this device's own passwords.
    assert_eq!(a.secret(&studio.to_string()).as_deref(), Some(INCOMING));
    // Turning it back on after a removal cancels any pending withdrawal.
    a.store
        .change_profile_sync_options(Changes {
            passwords: Some(true),
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!a.local().await.withdraw);
    let report = pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    assert_eq!(report.published, 2);
}

#[tokio::test]
async fn profile_vault_restart_keeps_revisions_and_does_not_republish() {
    let dir = tempfile::tempdir().unwrap();
    let cloud = Cloud::default();
    let studio = shared(1);
    let accounts = [(
        account(&studio.to_string(), "imap.studio.test", false),
        studio,
    )];
    let secrets = [(studio.to_string(), INCOMING.to_owned())];
    let a = device(
        Some(dir.path()),
        &accounts,
        false,
        &[(&secrets[0].0, &secrets[0].1)],
    )
    .await;
    pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    let saved = a.local().await;
    let keychain = a.keychain.clone();
    drop(a);
    let store = Store::open(dir.path().join("cache.sqlite")).unwrap();
    let reopened = Device {
        store,
        credentials: Credentials::with_backend(CredentialScope::Legacy, keychain.clone()),
        keychain,
    };
    assert_eq!(reopened.local().await, saved);
    let creates = cloud.creates.load(Ordering::SeqCst);
    let report = pass(
        &reopened,
        &cloud,
        &Servers::default(),
        &History::default(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(report, Report::default());
    assert_eq!(cloud.creates.load(Ordering::SeqCst), creates);
    // A failed write leaves the saved revisions for the next attempt.
    reopened
        .keychain
        .0
        .lock()
        .unwrap()
        .insert(studio.to_string(), "changed".into());
    *cloud.fail_create.lock().unwrap() = true;
    assert!(
        pass(
            &reopened,
            &cloud,
            &Servers::default(),
            &History::default(),
            false
        )
        .await
        .is_err()
    );
    assert_eq!(reopened.local().await, saved);
    *cloud.fail_create.lock().unwrap() = false;
    let report = pass(
        &reopened,
        &cloud,
        &Servers::default(),
        &History::default(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(report.published, 1);
}

#[tokio::test]
async fn profile_vault_concurrent_first_devices_converge_on_one_key() {
    let cloud = Cloud::racing();
    let (studio, home) = (shared(1), shared(2));
    let a = device(
        None,
        &[(
            account(&studio.to_string(), "imap.studio.test", false),
            studio,
        )],
        false,
        &[(&studio.to_string(), INCOMING)],
    )
    .await;
    let b = device(
        None,
        &[(account(&home.to_string(), "imap.home.test", false), home)],
        false,
        &[(&home.to_string(), "home password")],
    )
    .await;
    let servers = Servers::default();
    let history = History::default();
    // Both first devices list an empty vault and create keys concurrently.
    let (first, second) = tokio::join!(
        pass(&a, &cloud, &servers, &history, false),
        pass(&b, &cloud, &servers, &history, false)
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(
        cloud.key_creates.load(Ordering::SeqCst),
        2,
        "both devices raced to create the first key"
    );
    // Whichever key each device saw, later passes converge on one key and
    // one vault holding both devices' passwords.
    for device in [&a, &b, &a] {
        pass(device, &cloud, &servers, &history, false)
            .await
            .unwrap();
    }
    let (key, vault) = opened(&cloud);
    assert_eq!(
        secret_in(&key, &vault, studio, Field::Incoming).as_deref(),
        Some(INCOMING)
    );
    assert_eq!(
        secret_in(&key, &vault, home, Field::Incoming).as_deref(),
        Some("home password")
    );

    // Two keys created at once: the vault sealed under the losing key is
    // re-sealed under the canonical one, and the loser is deleted.
    let cloud = Cloud::default();
    let scope = binding();
    let winner = Key::new(
        scope.profile,
        scope.generation,
        Uuid::from_u128(1),
        1,
        [7; 32],
    )
    .unwrap();
    let loser = Key::new(
        scope.profile,
        scope.generation,
        Uuid::from_u128(2),
        1,
        [9; 32],
    )
    .unwrap();
    for key in [&winner, &loser] {
        cloud
            .create(
                scope_of(),
                NewFile {
                    kind: Kind::Key { sequence: 1 },
                    key: key.id(),
                    bytes: key.encode().unwrap(),
                },
            )
            .await
            .unwrap();
    }
    let endpoint = codec::endpoint(
        &portable(&account(&home.to_string(), "imap.home.test", false), home).unwrap(),
        Field::Incoming,
    );
    let slot = codec::Slot {
        account: home,
        field: Field::Incoming,
        revision: 1,
        endpoint: &endpoint,
    };
    let vault = Vault {
        profile: scope.profile,
        generation: scope.generation,
        key: loser.id(),
        revision: 1,
        minor: 0,
        entries: vec![codec::Entry {
            account: home,
            field: Field::Incoming,
            revision: 1,
            device: Uuid::from_u128(99),
            value: Value::Sealed {
                sealed: codec::seal(&loser, &slot, b"home password", [3; 12]).unwrap(),
                endpoint: endpoint.clone(),
            },
        }],
    };
    cloud
        .create(
            scope_of(),
            NewFile {
                kind: Kind::Vault { revision: 1 },
                key: loser.id(),
                bytes: vault.encode().unwrap(),
            },
        )
        .await
        .unwrap();
    pass(&a, &cloud, &servers, &history, false).await.unwrap();
    let (key, vault) = opened(&cloud);
    assert_eq!(key.id(), winner.id());
    assert_eq!(
        secret_in(&key, &vault, home, Field::Incoming).as_deref(),
        Some("home password")
    );
    assert_eq!(
        secret_in(&key, &vault, studio, Field::Incoming).as_deref(),
        Some(INCOMING)
    );
}

fn scope_of() -> Scope {
    let binding = binding();
    Scope {
        profile: binding.profile,
        generation: binding.generation,
    }
}

#[tokio::test]
async fn profile_vault_wrong_key_or_other_endpoint_is_never_imported_or_tested() {
    let cloud = Cloud::default();
    let studio = shared(1);
    let a = device(
        None,
        &[(
            account(&studio.to_string(), "imap.studio.test", false),
            studio,
        )],
        false,
        &[(&studio.to_string(), INCOMING)],
    )
    .await;
    pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    // Replace the key material while keeping its identity: authentication fails.
    let (key_file, _) = cloud.of(true).remove(0);
    let scope = binding();
    let impostor = Key::new(scope.profile, scope.generation, key_file.key, 1, [5; 32]).unwrap();
    cloud.replace(&key_file.id, impostor.encode().unwrap());
    let b = device(
        None,
        &[(account("b-native", "imap.studio.test", false), studio)],
        true,
        &[],
    )
    .await;
    let servers = accepting(&[INCOMING]);
    let report = pass(&b, &cloud, &servers, &History::default(), false)
        .await
        .unwrap();
    assert!(report.unreadable > 0 && report.imported == 0);
    assert!(servers.attempts.lock().unwrap().is_empty());
    assert!(b.secret("b-native").is_none());
    assert_eq!(b.reconnecting().await.len(), 1);
    // The publishing device writes its password again under a usable key.
    pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    let report = pass(&b, &cloud, &servers, &History::default(), false)
        .await
        .unwrap();
    assert_eq!(report.imported, 1);

    // An account whose server differs from the published endpoint waits for
    // reviewed reconnection and its server is never sent the password.
    let c = device(
        None,
        &[(account("c-native", "imap.elsewhere.test", false), studio)],
        true,
        &[],
    )
    .await;
    let servers = accepting(&[INCOMING]);
    let report = pass(&c, &cloud, &servers, &History::default(), false)
        .await
        .unwrap();
    assert_eq!((report.imported, report.waiting), (0, 1));
    assert!(servers.attempts.lock().unwrap().is_empty());
    assert!(c.secret("c-native").is_none());
}

#[tokio::test]
async fn profile_vault_history_tombstones_come_from_the_shared_worker() {
    let dir = tempfile::tempdir().unwrap();
    let mut replica = crate::profile_sync::replica::Replica::open(
        dir.path().join("history.sqlite"),
        binding(),
        crate::profile_sync::journal::Journal::open(None).unwrap(),
    )
    .await
    .unwrap();
    let (removed, kept) = (shared(1), shared(2));
    replica
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: 0,
            changes: vec![shep_profile_core::Change {
                action: shep_profile_core::Action::AccountRemoved { id: removed },
                extra: Default::default(),
            }],
            resolutions: vec![],
        })
        .await
        .unwrap();
    assert!(Tombstones::removed(&replica, removed).await.unwrap());
    assert!(!Tombstones::removed(&replica, kept).await.unwrap());
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_vault_newer_minor_version_is_read_only() {
    let cloud = Cloud::default();
    let studio = shared(1);
    let a = device(
        None,
        &[(
            account(&studio.to_string(), "imap.studio.test", false),
            studio,
        )],
        false,
        &[(&studio.to_string(), INCOMING)],
    )
    .await;
    pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    let (file, bytes) = cloud.of(false).remove(0);
    let newer = String::from_utf8(bytes)
        .unwrap()
        .replace("\"minor\":0", "\"minor\":1");
    cloud.replace(&file.id, newer.into_bytes());
    a.keychain
        .0
        .lock()
        .unwrap()
        .insert(studio.to_string(), "changed".into());
    let creates = cloud.creates.load(Ordering::SeqCst);
    let report = pass(&a, &cloud, &Servers::default(), &History::default(), false)
        .await
        .unwrap();
    assert!(report.read_only && report.published == 0);
    assert_eq!(cloud.creates.load(Ordering::SeqCst), creates);
}
