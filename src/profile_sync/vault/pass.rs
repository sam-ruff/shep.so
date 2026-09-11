//! One reconcile pass: merge every vault file, publish or remove this device's
//! entries, then hand received passwords back for a tested activation.
use super::*;
use rand::{RngCore, rngs::OsRng};
use secrecy::ExposeSecret;
use shep_profile_core::vault::{self as codec, Entry, Key, Value, Vault};
use zeroize::Zeroizing;

type Slot = (Uuid, Field);
type Merged = BTreeMap<Slot, (Entry, Uuid)>;

enum Write {
    Seal {
        revision: u64,
        endpoint: String,
        secret: SecretString,
    },
    Remove,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Publish(u64),
    Confirm(u64),
    Import(u64),
    /// This revision failed its connection test; only a retry tests it again.
    Held,
    Wait,
    Keep,
}

/// Per-field rule. `current` is this device's keychain value and `opened` the
/// merged winner's plaintext when its key and context verify.
fn decide(
    current: Option<&[u8]>,
    winner: Option<(&Entry, Option<&[u8]>)>,
    synced: Synced,
    endpoint: &str,
    retry: bool,
) -> Decision {
    let Some((entry, opened)) = winner else {
        return match current {
            Some(_) => Decision::Publish(synced.revision + 1),
            None => Decision::Keep,
        };
    };
    let next = entry.revision.max(synced.revision) + 1;
    if next > codec::MAX_REVISION {
        return Decision::Wait;
    }
    match (&entry.value, opened) {
        (Value::Removed, _) => match current {
            Some(_) => Decision::Publish(next),
            None => Decision::Keep,
        },
        (Value::Sealed { .. }, None) => match current {
            Some(_) => Decision::Publish(next),
            None => Decision::Wait,
        },
        (
            Value::Sealed {
                endpoint: published,
                ..
            },
            Some(value),
        ) => {
            if published != endpoint {
                // Another endpoint waits for the reviewed reconnection rules.
                return match current {
                    Some(_) if entry.revision <= synced.revision => Decision::Publish(next),
                    _ => Decision::Wait,
                };
            }
            if current == Some(value) {
                return Decision::Confirm(entry.revision);
            }
            if current.is_some() && entry.revision <= synced.revision {
                return Decision::Publish(next);
            }
            if retry || entry.revision != synced.failed {
                Decision::Import(entry.revision)
            } else {
                Decision::Held
            }
        }
    }
}

struct Snapshot {
    keys: Vec<(RemoteFile, Option<Key>)>,
    vaults: Vec<(RemoteFile, Vault)>,
}
impl Snapshot {
    fn key(&self, id: Uuid) -> Option<&Key> {
        self.keys
            .iter()
            .find_map(|(file, key)| (file.key == id).then_some(key.as_ref()).flatten())
    }
    fn canonical(&self) -> Option<&Key> {
        let usable = self
            .keys
            .iter()
            .filter_map(|(_, key)| key.as_ref().map(|k| (k.id(), k.sequence())));
        self.key(codec::canonical(usable)?.0)
    }
    fn max_sequence(&self) -> u32 {
        self.keys
            .iter()
            .filter_map(|(file, _)| match file.kind {
                Kind::Key { sequence } => Some(sequence),
                Kind::Vault { .. } => None,
            })
            .max()
            .unwrap_or(0)
    }
    fn open(&self, entry: &Entry, key: Uuid) -> Option<Zeroizing<Vec<u8>>> {
        let Value::Sealed { sealed, .. } = &entry.value else {
            return None;
        };
        codec::open(self.key(key)?, &entry.slot()?, sealed).ok()
    }
    /// Keys that neither seal a listed vault nor are the canonical key.
    fn strays(&self) -> Vec<&RemoteFile> {
        let canonical = self.canonical().map(Key::id);
        self.keys
            .iter()
            .map(|(file, _)| file)
            .filter(|file| {
                Some(file.key) != canonical && !self.vaults.iter().any(|(v, _)| v.key == file.key)
            })
            .collect()
    }
}

async fn load(
    ctx: &Context<'_>,
    scope: Scope,
    files: Vec<RemoteFile>,
) -> anyhow::Result<(Snapshot, usize)> {
    let mut snapshot = Snapshot {
        keys: Vec::new(),
        vaults: Vec::new(),
    };
    let mut unreadable = 0;
    for file in files {
        let bytes = Zeroizing::new(ctx.control.read(ctx.remote.download(scope, &file)).await?);
        match file.kind {
            Kind::Key { sequence } => {
                let key = match Key::decode(&bytes, scope.profile, scope.generation) {
                    Ok(key) if key.id() == file.key && key.sequence() == sequence => Some(key),
                    Err(codec::Error::Upgrade) => return Err(codec::Error::Upgrade.into()),
                    _ => None,
                };
                snapshot.keys.push((file, key));
            }
            Kind::Vault { revision } => {
                match Vault::decode(&bytes, scope.profile, scope.generation) {
                    Ok(vault) if vault.key == file.key && vault.revision == revision => {
                        snapshot.vaults.push((file, vault));
                    }
                    Err(codec::Error::Upgrade) => return Err(codec::Error::Upgrade.into()),
                    // Damaged files are left untouched and never merged.
                    _ => unreadable += 1,
                }
            }
        }
    }
    Ok((snapshot, unreadable))
}

fn nonce() -> [u8; codec::NONCE_BYTES] {
    let mut nonce = [0; codec::NONCE_BYTES];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

/// Create a key, then list again: if another device created one at the same
/// time, every device converges on the canonical key and the loser is deleted.
async fn create_key(ctx: &Context<'_>, scope: Scope, sequence: u32) -> anyhow::Result<Key> {
    let mut material = Zeroizing::new([0; codec::KEY_BYTES]);
    OsRng.fill_bytes(&mut material[..]);
    let key = Key::new(
        scope.profile,
        scope.generation,
        Uuid::new_v4(),
        sequence,
        *material,
    )?;
    let created = ctx
        .remote
        .create(
            scope,
            NewFile {
                kind: Kind::Key { sequence },
                key: key.id(),
                bytes: key.encode()?,
            },
        )
        .await?;
    let listed = ctx.remote.list(scope).await?;
    let keys = listed.iter().filter_map(|file| match file.kind {
        Kind::Key { sequence } => Some((file.key, sequence)),
        Kind::Vault { .. } => None,
    });
    let Some((winner, _)) = codec::canonical(keys) else {
        return Ok(key);
    };
    if winner == key.id() {
        return Ok(key);
    }
    let Some(file) = listed
        .iter()
        .find(|file| file.key == winner && matches!(file.kind, Kind::Key { .. }))
    else {
        return Ok(key);
    };
    let bytes = Zeroizing::new(ctx.remote.download(scope, file).await?);
    match Key::decode(&bytes, scope.profile, scope.generation) {
        Ok(other) if other.id() == winner => {
            ctx.remote.delete(&created).await?;
            Ok(other)
        }
        _ => Ok(key),
    }
}

enum Next {
    Seal {
        revision: u64,
        device: Uuid,
        endpoint: String,
        secret: Zeroizing<Vec<u8>>,
    },
    Removed {
        revision: u64,
        device: Uuid,
    },
}

/// Write the merged vault under the canonical key (a new one when a password
/// was removed), then delete the merged files and unreferenced keys.
async fn write(
    ctx: &Context<'_>,
    plan: &Plan,
    snapshot: &Snapshot,
    merged: &Merged,
    writes: &BTreeMap<Slot, Write>,
) -> anyhow::Result<bool> {
    let scope = plan.scope();
    let mut next = BTreeMap::new();
    let mut rotate = false;
    for (slot, (entry, key)) in merged {
        let value = match (writes.get(slot), &entry.value) {
            (Some(Write::Seal { .. }), _) => continue,
            (Some(Write::Remove), Value::Sealed { .. }) => {
                rotate = true;
                Next::Removed {
                    revision: entry.revision + 1,
                    device: plan.device,
                }
            }
            (_, Value::Removed) => Next::Removed {
                revision: entry.revision,
                device: entry.device,
            },
            (None, Value::Sealed { endpoint, .. }) => match snapshot.open(entry, *key) {
                Some(secret) => Next::Seal {
                    revision: entry.revision,
                    device: entry.device,
                    endpoint: endpoint.clone(),
                    secret,
                },
                // Its key is gone; the publishing device will write it again.
                None => continue,
            },
        };
        next.insert(*slot, value);
    }
    for (slot, write) in writes {
        if let Write::Seal {
            revision,
            endpoint,
            secret,
        } = write
        {
            next.insert(
                *slot,
                Next::Seal {
                    revision: *revision,
                    device: plan.device,
                    endpoint: endpoint.clone(),
                    secret: Zeroizing::new(secret.expose_secret().as_bytes().to_vec()),
                },
            );
        }
    }
    if !next.values().any(|n| matches!(n, Next::Seal { .. })) {
        // No password remains, so nothing needs a key or a vault.
        for (file, _) in &snapshot.vaults {
            ctx.remote.delete(file).await?;
        }
        for (file, _) in &snapshot.keys {
            ctx.remote.delete(file).await?;
        }
        return Ok(rotate);
    }
    let key = match (rotate, snapshot.canonical()) {
        (false, Some(key)) => key.clone(),
        _ => create_key(ctx, scope, snapshot.max_sequence() + 1).await?,
    };
    let revision = snapshot
        .vaults
        .iter()
        .map(|(_, v)| v.revision)
        .max()
        .unwrap_or(0)
        + 1;
    let mut entries = Vec::with_capacity(next.len());
    for ((account, field), value) in next {
        entries.push(match value {
            Next::Seal {
                revision,
                device,
                endpoint,
                secret,
            } => {
                let slot = codec::Slot {
                    account,
                    field,
                    revision,
                    endpoint: &endpoint,
                };
                let sealed = codec::seal(&key, &slot, &secret, nonce())?;
                Entry {
                    account,
                    field,
                    revision,
                    device,
                    value: Value::Sealed { endpoint, sealed },
                }
            }
            Next::Removed { revision, device } => Entry {
                account,
                field,
                revision,
                device,
                value: Value::Removed,
            },
        });
    }
    let vault = Vault {
        profile: scope.profile,
        generation: scope.generation,
        key: key.id(),
        revision,
        minor: 0,
        entries,
    };
    let created = ctx
        .remote
        .create(
            scope,
            NewFile {
                kind: Kind::Vault { revision },
                key: key.id(),
                bytes: vault.encode()?,
            },
        )
        .await?;
    for (file, _) in &snapshot.vaults {
        if file.id != created.id {
            ctx.remote.delete(file).await?;
        }
    }
    let listed = ctx.remote.list(scope).await?;
    for file in &listed {
        if matches!(file.kind, Kind::Key { .. })
            && file.key != key.id()
            && !listed
                .iter()
                .any(|v| matches!(v.kind, Kind::Vault { .. }) && v.key == file.key)
        {
            ctx.remote.delete(file).await?;
        }
    }
    Ok(rotate)
}

fn sealed(merged: &Merged, slot: &Slot) -> bool {
    merged
        .get(slot)
        .is_some_and(|(entry, _)| matches!(entry.value, Value::Sealed { .. }))
}

/// Publish, remove and confirm this device's slots. Imports are returned so
/// the engine can test and activate them under its account locks.
pub async fn reconcile(ctx: &Context<'_>) -> anyhow::Result<Outcome> {
    let Some(plan) = ctx.store.credential_plan().await? else {
        return Ok(Outcome::default());
    };
    for local in &plan.local.staged {
        for field in [Field::Incoming, Field::Smtp] {
            ctx.credentials.delete(&staged_key(local, field)).await?;
        }
        ctx.store
            .mark_credentials_staged(local.clone(), false)
            .await?;
    }
    // With the toggle off this device only ever removes its own entries.
    if !plan.publish && !plan.local.withdraw {
        return Ok(Outcome::default());
    }
    let scope = plan.scope();
    let mut report = Report::default();
    let files = ctx.control.read(ctx.remote.list(scope)).await?;
    let (snapshot, unreadable) = load(ctx, scope, files).await?;
    report.unreadable = unreadable;
    report.read_only = snapshot.vaults.iter().any(|(_, v)| v.minor > 0);
    let merged = codec::merge(snapshot.vaults.iter().map(|(_, v)| v));
    let own = |slot: &Slot| {
        merged.get(slot).is_some_and(|(e, _)| {
            e.device == plan.device && matches!(e.value, Value::Sealed { .. })
        })
    };
    let mut writes = BTreeMap::new();
    let mut confirmed = BTreeMap::new();
    let mut forget = BTreeSet::new();
    let mut imports = Vec::new();
    for (local, shared) in &plan.mapping {
        ctx.control.check()?;
        let removed = ctx.history.removed(*shared).await?;
        let account = plan
            .accounts
            .iter()
            .find(|a| &a.id == local)
            .filter(|_| !removed && !plan.suppressed.contains(shared));
        let Some(account) = account else {
            for field in [Field::Incoming, Field::Smtp] {
                let slot = (*shared, field);
                if sealed(&merged, &slot) && (removed || own(&slot)) {
                    writes.insert(slot, Write::Remove);
                }
            }
            forget.insert(*shared);
            continue;
        };
        if !plan.publish {
            continue;
        }
        let required = fields(account);
        for field in [Field::Incoming, Field::Smtp] {
            let slot = (*shared, field);
            if !required.contains(&field) && own(&slot) {
                writes.insert(slot, Write::Remove);
            }
        }
        let Ok(connection) = portable(account, *shared) else {
            report.waiting += 1;
            continue;
        };
        let reconnecting = plan.reconnect.contains(local);
        let mut pair = Vec::new();
        let mut revisions = Vec::new();
        let mut complete = true;
        for field in required {
            let slot = (*shared, field);
            let endpoint = codec::endpoint(&connection, field);
            let current = if reconnecting {
                None
            } else {
                ctx.credentials
                    .read_optional(&keychain_key(local, field))
                    .await?
            };
            let winner = merged.get(&slot);
            let opened = winner.and_then(|(entry, key)| snapshot.open(entry, *key));
            if winner.is_some_and(|(e, _)| matches!(e.value, Value::Sealed { .. }))
                && opened.is_none()
            {
                report.unreadable += 1;
            }
            let decision = decide(
                current.as_ref().map(|s| s.expose_secret().as_bytes()),
                winner.map(|(entry, _)| (entry, opened.as_ref().map(|v| &v[..]))),
                plan.local.synced(*shared, field),
                &endpoint,
                ctx.retry_failed,
            );
            match (decision, current) {
                (Decision::Publish(revision), Some(secret)) => {
                    pair.push((field, secret.clone()));
                    writes.insert(
                        slot,
                        Write::Seal {
                            revision,
                            endpoint,
                            secret,
                        },
                    );
                }
                (Decision::Confirm(revision), Some(secret)) => {
                    confirmed.insert(slot, revision);
                    pair.push((field, secret));
                }
                (Decision::Import(revision), _) => {
                    match opened
                        .as_ref()
                        .and_then(|value| String::from_utf8(value.to_vec()).ok())
                    {
                        Some(text) => {
                            pair.push((field, SecretString::from(text)));
                            revisions.push((field, revision));
                        }
                        None => complete = false,
                    }
                }
                (Decision::Keep, Some(secret)) => pair.push((field, secret)),
                (Decision::Held, _) => {
                    report.held += 1;
                    complete = false;
                }
                (Decision::Wait, _) => {
                    report.waiting += 1;
                    complete = false;
                }
                _ => complete = false,
            }
        }
        if !revisions.is_empty() {
            if complete {
                imports.push(Import {
                    local: local.clone(),
                    shared: *shared,
                    account: account.clone(),
                    pair,
                    revisions,
                });
            } else {
                report.waiting += 1;
            }
        }
    }
    let withdraw = plan.local.withdraw && !plan.publish;
    if withdraw {
        for slot in merged.keys() {
            if own(slot) {
                writes.insert(*slot, Write::Remove);
            }
        }
    }
    let mapped: BTreeSet<Uuid> = plan.mapping.values().copied().collect();
    let unmapped: BTreeSet<Uuid> = merged
        .keys()
        .map(|(account, _)| *account)
        .filter(|account| !mapped.contains(account))
        .collect();
    for account in unmapped {
        let slots = [(account, Field::Incoming), (account, Field::Smtp)];
        if slots.iter().any(|slot| sealed(&merged, slot)) && ctx.history.removed(account).await? {
            for slot in slots {
                if sealed(&merged, &slot) {
                    writes.insert(slot, Write::Remove);
                }
            }
        }
    }
    ctx.control.check()?;
    let needs_vault = !writes.is_empty()
        || snapshot.vaults.len() > 1
        || snapshot
            .vaults
            .iter()
            .any(|(file, _)| Some(file.key) != snapshot.canonical().map(Key::id))
        || merged.iter().any(|(slot, (entry, key))| {
            sealed(&merged, slot) && snapshot.open(entry, *key).is_none()
        });
    let mut written = false;
    if report.read_only {
        report.waiting += writes.len();
        writes.clear();
    } else if needs_vault {
        report.rotated = write(ctx, &plan, &snapshot, &merged, &writes).await?;
        written = true;
    } else {
        for file in snapshot.strays() {
            ctx.remote.delete(file).await?;
        }
    }
    let mut local = plan.local.clone();
    local.binding = Some(plan.binding.clone());
    local.device = Some(plan.device);
    if written {
        for (slot, write) in &writes {
            let key = slot_key(slot.0, slot.1);
            match write {
                Write::Seal { revision, .. } => {
                    report.published += 1;
                    local.slots.insert(
                        key,
                        Synced {
                            revision: *revision,
                            failed: 0,
                        },
                    );
                }
                Write::Remove => {
                    report.removed += 1;
                    local.slots.remove(&key);
                }
            }
        }
    }
    for ((account, field), revision) in confirmed {
        local
            .slots
            .entry(slot_key(account, field))
            .or_default()
            .revision = revision;
    }
    for account in forget {
        for field in [Field::Incoming, Field::Smtp] {
            local.slots.remove(&slot_key(account, field));
        }
    }
    if withdraw && !report.read_only {
        local.withdraw = false;
        local.slots.clear();
        report.withdrawn = true;
    }
    if local != plan.local {
        ctx.store
            .commit_credentials(plan.local.revision, local)
            .await?;
    }
    Ok(Outcome { report, imports })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(revision: u64, value: Value) -> Entry {
        Entry {
            account: Uuid::from_u128(1),
            field: Field::Incoming,
            revision,
            device: Uuid::from_u128(2),
            value,
        }
    }
    fn sealed_at(revision: u64, endpoint: &str) -> Entry {
        entry(
            revision,
            Value::Sealed {
                endpoint: endpoint.into(),
                sealed: "AQ==".into(),
            },
        )
    }
    const HERE: &str = "a";
    const ELSEWHERE: &str = "b";

    #[test]
    fn profile_vault_decisions_never_overwrite_newer_remote_passwords() {
        let at = |revision, failed| Synced { revision, failed };
        let remote = sealed_at(4, HERE);
        let open = Some((&remote, Some(&b"remote"[..])));
        // Nothing published yet: publish, or nothing to import.
        assert_eq!(
            decide(Some(b"mine"), None, at(0, 0), HERE, false),
            Decision::Publish(1)
        );
        assert_eq!(decide(None, None, at(0, 0), HERE, false), Decision::Keep);
        // Equal values only record the revision.
        assert_eq!(
            decide(Some(b"remote"), open, at(1, 0), HERE, false),
            Decision::Confirm(4)
        );
        // A newer remote revision is imported, never overwritten.
        assert_eq!(
            decide(Some(b"mine"), open, at(3, 0), HERE, false),
            Decision::Import(4)
        );
        assert_eq!(
            decide(None, open, at(9, 0), HERE, false),
            Decision::Import(4)
        );
        // A local change after the last sync publishes above the winner.
        assert_eq!(
            decide(Some(b"mine"), open, at(4, 0), HERE, false),
            Decision::Publish(5)
        );
        // A failed revision waits for an explicit retry.
        assert_eq!(
            decide(Some(b"mine"), open, at(3, 4), HERE, false),
            Decision::Held
        );
        assert_eq!(decide(None, open, at(0, 4), HERE, false), Decision::Held);
        assert_eq!(
            decide(None, open, at(0, 4), HERE, true),
            Decision::Import(4)
        );
        // Another endpoint never receives this password or replaces it.
        assert_eq!(
            decide(None, open, at(0, 0), ELSEWHERE, false),
            Decision::Wait
        );
        assert_eq!(
            decide(Some(b"mine"), open, at(3, 0), ELSEWHERE, false),
            Decision::Wait
        );
        assert_eq!(
            decide(Some(b"mine"), open, at(4, 0), ELSEWHERE, false),
            Decision::Publish(5)
        );
        // Unreadable or removed winners are replaced by a local password.
        let lost = sealed_at(6, HERE);
        assert_eq!(
            decide(Some(b"mine"), Some((&lost, None)), at(2, 0), HERE, false),
            Decision::Publish(7)
        );
        assert_eq!(
            decide(None, Some((&lost, None)), at(2, 0), HERE, false),
            Decision::Wait
        );
        let removed = entry(8, Value::Removed);
        assert_eq!(
            decide(Some(b"mine"), Some((&removed, None)), at(2, 0), HERE, false),
            Decision::Publish(9)
        );
        assert_eq!(
            decide(None, Some((&removed, None)), at(2, 0), HERE, false),
            Decision::Keep
        );
    }
}
