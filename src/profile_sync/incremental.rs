//! Enrolled-history pulls through the shared discovery catalog. The catalog
//! owns Drive change tokens, page progress and verified immutable records; this
//! module copies its verified observations into the device's enrolled history
//! from a durable per-copy cursor. A full listing remains the recovery path.
use super::{
    Binding, Record, control::Control, drive::Session, journal::CopyIdentity, paths::Paths,
};
use anyhow::Context;
use shep_profile_core::{
    drive::{
        self,
        catalog::{Discovery, Error as CatalogError, Phase, Scope, Snapshot, State},
    },
    history,
};
use std::{path::PathBuf, sync::Arc};

/// Where this workspace keeps the catalog for one Google account/namespace.
#[derive(Clone)]
pub(crate) struct Location {
    path: PathBuf,
    key: Option<Arc<crate::cache_cipher::Key>>,
    scope: Scope,
}
impl Paths {
    pub(crate) fn catalog_location(&self, transport: &Binding) -> anyhow::Result<Location> {
        transport.validate()?;
        let scope = Scope {
            namespace: transport.namespace().into(),
            principal: transport.identity().into(),
        };
        Ok(Location {
            path: self.catalog(&scope)?,
            key: self.key.clone(),
            scope,
        })
    }
}
impl Location {
    pub(super) fn check_session(&self, session: &Session) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.scope.namespace == session.binding().namespace()
                && self.scope.principal == session.binding().identity(),
            "The profile belongs to another Google account or application. Reconnect its original account."
        );
        Ok(())
    }
    async fn open(&self) -> anyhow::Result<Discovery> {
        Ok(Discovery::open_with(
            self.path.clone(),
            self.scope.clone(),
            crate::cache_cipher::profile_connections(self.key.clone()),
        )
        .await?)
    }
}

/// A completed, verified catalog scan for one enrolled profile. Publication
/// checks remote identities against this same completed state. The catalog
/// owner is released after the copy so native discovery pages stay available.
pub struct Verified {
    location: Location,
    snapshot: Snapshot,
    revision: u64,
}
impl Verified {
    pub(super) fn check_session(&self, session: &Session) -> anyhow::Result<()> {
        self.location.check_session(session)
    }
    pub(super) fn binding(&self) -> &history::Binding {
        &self.snapshot.binding
    }
    pub(super) fn records(&self) -> u64 {
        self.snapshot.profile.operations
    }
    /// Whether the exact record already exists in the verified inventory. A
    /// changed catalog or differing bytes are errors; absence is `false`.
    pub(super) async fn observed(&self, record: &Record) -> anyhow::Result<bool> {
        let original = history::Record {
            position: 0,
            operation: record.operation().operation,
            record: String::from_utf8(record.bytes().to_vec())?,
        };
        let catalog = self.location.open().await?;
        let result = async {
            let state = catalog.state().await?;
            anyhow::ensure!(
                state.revision == self.revision && state.phase == Phase::Complete,
                "The profile changed after discovery. Pull again before publishing its saved edits."
            );
            match catalog
                .verify_original(self.snapshot.clone(), original)
                .await
            {
                Ok(()) => Ok(true),
                Err(CatalogError::Missing) => Ok(false),
                Err(error) => Err(error.into()),
            }
        }
        .await;
        close(catalog).await?;
        result
    }
}

async fn close(catalog: Discovery) -> anyhow::Result<()> {
    catalog
        .close()
        .await
        .context("Could not finish saving profile discovery")
}

/// Token/pagination failures that a complete listing can repair. Any other
/// error keeps the saved catalog progress and surfaces as it is.
fn rescan_required(error: &CatalogError, before: &State) -> bool {
    match error {
        // With no pending download or staged page, the only request in the
        // changes phase is the change poll itself: Google rejected its token.
        CatalogError::Provider(drive::Error::Http(400 | 404 | 410)) => {
            before.phase == Phase::Changes && before.pending == 0
        }
        CatalogError::Integrity | CatalogError::Missing => true,
        _ => false,
    }
}

/// Resume saved progress, poll the change stream from the persisted token, or
/// fall back once to a full listing. Only a completed state is returned.
async fn complete(
    catalog: &Discovery,
    drive: &drive::Drive,
    control: &Control,
    full: bool,
) -> anyhow::Result<State> {
    let mut state = catalog.state().await?;
    if full {
        state = catalog.refresh(state.revision, true).await?;
    } else if state.error.is_some() {
        state = catalog.retry(state.revision).await?;
    } else if state.phase == Phase::Complete {
        state = catalog.refresh(state.revision, false).await?;
    }
    let mut rescanned = full;
    while state.phase != Phase::Complete {
        control.check()?;
        // The shared owner drains any SQL admitted before GET cancellation.
        match control
            .read(async { Ok(catalog.advance(drive).await) })
            .await?
        {
            Ok(next) => state = next,
            Err(error) if !rescanned && rescan_required(&error, &state) => {
                rescanned = true;
                let current = catalog.state().await?;
                state = catalog.refresh(current.revision, true).await?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(state)
}

pub(super) struct Copied {
    pub verified: Verified,
    pub imported: u64,
}

/// The enrolled history receiving catalog records, with the transport journal
/// that keeps its copy cursor. The replica owns both workers.
pub(super) struct Target<'a> {
    pub binding: &'a history::Binding,
    pub journal: &'a super::journal::Journal,
    pub history: &'a history::Worker,
}

/// Bring the enrolled history up to the catalog's verified observation of this
/// profile. Records are imported one at a time and the cursor is saved after
/// each import, so an interrupted pass or lost checkpoint replays exact bytes.
pub(super) async fn copy(
    location: &Location,
    session: &Session,
    target: &Target<'_>,
    control: &Control,
) -> anyhow::Result<Copied> {
    control.check()?;
    location.check_session(session)?;
    let drive = control.read(session.catalog_drive()).await?;
    let catalog = location.open().await?;
    let result = async {
        let (snapshot, revision) = observe(&catalog, &drive, target.binding, control).await?;
        let imported = import(&catalog, &snapshot, session.binding(), target, control).await?;
        Ok(Copied {
            verified: Verified {
                location: location.clone(),
                snapshot,
                revision,
            },
            imported,
        })
    }
    .await;
    // Accepted catalog writes drain before the owner is released, whether or
    // not the copy succeeded.
    close(catalog).await?;
    result
}

/// Complete the catalog and freeze this profile's verified observation.
async fn observe(
    catalog: &Discovery,
    drive: &drive::Drive,
    binding: &history::Binding,
    control: &Control,
) -> anyhow::Result<(Snapshot, u64)> {
    let mut state = complete(catalog, drive, control, false).await?;
    let mut rescanned = false;
    let snapshot = loop {
        anyhow::ensure!(
            state.phase == Phase::Complete && state.error.is_none() && state.pending == 0,
            "Profile discovery did not finish. Retry the check; the local setup was kept."
        );
        match catalog
            .latest_snapshot(binding.profile, binding.generation)
            .await
        {
            Ok(snapshot) => break snapshot,
            // A listed profile whose observation history disagrees with its
            // summary was rebuilt: one full listing restores it before copying.
            Err(CatalogError::Changed)
                if !rescanned && listed(catalog, binding, control).await? =>
            {
                rescanned = true;
                state = complete(catalog, drive, control, true).await?;
            }
            Err(CatalogError::Changed) => anyhow::bail!(
                "The shared profile is missing from Drive. Local accounts and changes have been kept; check the connected Google account."
            ),
            Err(CatalogError::History(history::Error::Incomplete)) => anyhow::bail!(
                "This shared profile is incomplete or removed. Local accounts have been kept for review."
            ),
            Err(error) => return Err(error.into()),
        }
    };
    anyhow::ensure!(
        &snapshot.binding == binding,
        "The discovered profile belongs to another account or application."
    );
    Ok((snapshot, state.revision))
}

/// Export records after the saved cursor into the enrolled history.
async fn import(
    catalog: &Discovery,
    snapshot: &Snapshot,
    transport: &Binding,
    target: &Target<'_>,
    control: &Control,
) -> anyhow::Result<u64> {
    let source = catalog.source_device(snapshot.clone()).await?;
    let device = state_of(target.history).await?.device;
    let identity = CopyIdentity {
        profile: target.binding.profile,
        generation: target.binding.generation,
        source,
        device,
    };
    let mut position = target.journal.copy_position(transport, identity).await?;
    let mut imported = 0;
    loop {
        control.check()?;
        let Some(record) = catalog.export_record(snapshot.clone(), position).await? else {
            break;
        };
        anyhow::ensure!(
            record.position > position,
            "Profile discovery returned an earlier record. Retry the check."
        );
        target
            .history
            .request(history::Command::Import {
                record: record.record,
            })
            .await?;
        target
            .journal
            .checkpoint_copy(transport, identity, record.position)
            .await?;
        position = record.position;
        imported += 1;
    }
    Ok(imported)
}

/// Whether the completed catalog lists this profile at all, one summary page
/// at a time. Summaries are small; an account has few profiles.
async fn listed(
    catalog: &Discovery,
    binding: &history::Binding,
    control: &Control,
) -> anyhow::Result<bool> {
    let key = format!("{}:{}", binding.profile, binding.generation);
    let mut after = None;
    loop {
        control.check()?;
        let page = catalog.profiles(after).await?;
        if page.iter().any(|profile| profile.cursor() == key) {
            return Ok(true);
        }
        match page.last() {
            Some(last) if page.len() == 50 => after = Some(last.cursor()),
            _ => return Ok(false),
        }
    }
}

async fn state_of(history: &history::Worker) -> anyhow::Result<history::State> {
    match history.request(history::Command::State).await? {
        history::Reply::State(state) => Ok(state),
        _ => anyhow::bail!("The profile worker returned an unexpected state."),
    }
}

#[cfg(test)]
mod tests;
