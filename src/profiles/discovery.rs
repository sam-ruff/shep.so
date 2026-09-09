//! One desktop discovery owner. The provider queue drives one bounded step at a
//! time; cached mail, drafts and preferences use their existing independent queues.
use super::{enrollment, publication};
use crate::model::Preferences;
use anyhow::{Result, ensure};
use serde::Serialize;
use shep_profile_core::drive::{
    Drive,
    catalog::{Discovery, Profile, Scope, State},
};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Grant {
    revision: u64,
    id: String,
    client: String,
    principal: String,
    allowed: bool,
}
impl Grant {
    pub fn from_preferences(prefs: &Preferences) -> Self {
        Self {
            revision: prefs.google_lifecycle.revision,
            id: prefs.google_grant.id.clone(),
            client: prefs.active_google_client().into(),
            principal: prefs.google_connection_id.clone(),
            allowed: !prefs.google_lifecycle.disconnected
                && !prefs.google_lifecycle.cleanup_pending
                && prefs.google_grant.access.known
                && prefs.google_grant.access.drive,
        }
    }
    pub fn check(&self, prefs: &Preferences) -> Result<()> {
        ensure!(
            *self == Self::from_preferences(prefs),
            "Google setup changed. Reopen Profiles and sync for the current account."
        );
        ensure!(
            self.allowed && !self.id.is_empty() && self.principal.starts_with("drive:"),
            "Sign in to Google with Drive access in Preferences before discovering profiles."
        );
        Ok(())
    }
    pub fn client_id(&self) -> &str {
        &self.client
    }
    pub fn principal(&self) -> &str {
        &self.principal
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Load,
    Open { namespace: String },
    Page { after: Option<String> },
    Advance,
    Retry { revision: u64 },
    Refresh { revision: u64, full: bool },
    Publication(publication::Command),
    Enrollment(enrollment::Command),
    Close,
}
#[derive(Clone, Debug)]
pub struct Request {
    pub panel: Uuid,
    pub serial: u64,
    pub grant: Grant,
    pub action: Action,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Observation {
    pub namespace: String,
    pub state: Option<State>,
    pub rows: Vec<Profile>,
    pub after: Option<String>,
    pub error: Option<String>,
    pub publication: publication::Observation,
    pub enrollment: enrollment::Observation,
}
pub struct Session {
    pub panel: Uuid,
    pub grant: Grant,
    pub namespace: String,
    catalog: Discovery,
    after: Option<String>,
    history_root: PathBuf,
    publication: publication::Observation,
    enrollment: enrollment::Observation,
}
impl Session {
    pub async fn open(root: PathBuf, panel: Uuid, grant: Grant, drive: &Drive) -> Result<Self> {
        ensure!(
            !panel.is_nil() && drive.principal() == grant.principal(),
            "Google profile identity changed. Reopen discovery."
        );
        let scope = Scope {
            namespace: drive.namespace().into(),
            principal: drive.principal().into(),
        };
        let path = root.join(format!("{}.sqlite", scope.storage_key()?));
        Ok(Self {
            panel,
            grant,
            namespace: scope.namespace.clone(),
            history_root: root.join("histories"),
            publication: publication::Observation {
                next_id: Some(Uuid::new_v4()),
                ..Default::default()
            },
            enrollment: enrollment::Observation {
                next_id: Some(Uuid::new_v4()),
                ..Default::default()
            },
            catalog: Discovery::open(path, scope).await?,
            after: None,
        })
    }
    pub async fn observe(&self, error: Option<String>) -> Result<Observation> {
        Ok(Observation {
            namespace: self.namespace.clone(),
            state: Some(self.catalog.state().await?),
            rows: self.catalog.profiles(self.after.clone()).await?,
            after: self.after.clone(),
            error,
            publication: self.publication.clone(),
            enrollment: self.enrollment.clone(),
        })
    }
    pub async fn run(&mut self, action: Action, drive: Option<&Drive>) -> Result<Observation> {
        let result: Result<()> = async {
            match action {
                Action::Page { after } => {
                    // Validate before changing the cursor. Failed navigation
                    // retains the previous page for a visible, same-page retry.
                    self.catalog.profiles(after.clone()).await?;
                    self.after = after;
                }
                Action::Advance => {
                    self.catalog
                        .advance(drive.ok_or_else(|| {
                            anyhow::anyhow!("Reconnect Google before continuing discovery.")
                        })?)
                        .await?;
                }
                Action::Retry { revision } => {
                    self.catalog.retry(revision).await?;
                }
                Action::Refresh { revision, full } => {
                    self.catalog.refresh(revision, full).await?;
                    self.after = None;
                }
                _ => anyhow::bail!("Reopen profile discovery before continuing."),
            }
            Ok(())
        }
        .await;
        self.observe(result.err().map(|error| error.to_string()))
            .await
    }
    pub fn scope(&self) -> Scope {
        Scope {
            namespace: self.namespace.clone(),
            principal: self.grant.principal().into(),
        }
    }
    pub async fn publication_network(
        &self,
        store: &crate::store::Store,
        command: &publication::Command,
    ) -> Result<bool> {
        if let publication::Command::Step { id } = command {
            return Ok(store
                .publication_review(self.scope().storage_key()?, *id)
                .await?
                .phase
                == publication::Phase::Uploading);
        }
        Ok(false)
    }
    pub async fn run_publication(
        &mut self,
        store: &crate::store::Store,
        command: publication::Command,
        drive: Option<&Drive>,
    ) -> Result<Observation> {
        let after = if let publication::Command::Accounts { after, .. } = &command {
            *after
        } else {
            0
        };
        let prepare = if let publication::Command::Prepare { id, .. } = &command {
            Some(*id)
        } else {
            None
        };
        let result = publication::run(
            store,
            self.scope(),
            &self.history_root,
            &self.catalog,
            drive,
            command,
        )
        .await;
        let (review, error) = match result {
            Ok(review) => (review, None),
            Err(error) => (
                store
                    .publication_current(self.scope().storage_key()?)
                    .await?,
                Some(error.to_string()),
            ),
        };
        // Observe the saved receipt even when its caller lost an earlier reply.
        // A new form gets a backend-issued UUID; iced never needs OS randomness.
        if prepare.is_some()
            && prepare == self.publication.next_id
            && review.as_ref().is_some_and(|r| Some(r.id) == prepare)
        {
            self.publication.next_id = Some(Uuid::new_v4());
        }
        let after = if error.is_none() { after } else { 0 };
        let rows = if let Some(review) = &review {
            store
                .publication_accounts(self.scope().storage_key()?, review.id, after)
                .await?
        } else {
            vec![]
        };
        self.publication.review = review;
        self.publication.rows = rows;
        self.publication.after = after;
        self.observe(error).await
    }
    pub async fn run_enrollment(
        &mut self,
        store: &crate::store::Store,
        command: enrollment::Command,
    ) -> Result<Observation> {
        use crate::store::profile_enrollment as sql;
        let after = if let enrollment::Command::Rows { after, .. } = &command {
            *after
        } else {
            0
        };
        let prepare = if let enrollment::Command::Prepare { id, .. } = &command {
            Some(*id)
        } else {
            None
        };
        let key = self.scope().storage_key()?;
        let result = enrollment::run(
            store,
            self.scope(),
            &self.history_root,
            &self.catalog,
            command,
        )
        .await;
        let (review, error) = match result {
            Ok(review) => (review, None),
            Err(error) => {
                let key = key.clone();
                (
                    store.run(move |db| sql::current(db, &key)).await?,
                    Some(error.to_string()),
                )
            }
        };
        if prepare.is_some()
            && prepare == self.enrollment.next_id
            && review.as_ref().is_some_and(|r| Some(r.id) == prepare)
        {
            self.enrollment.next_id = Some(Uuid::new_v4());
        }
        let after = if error.is_none() { after } else { 0 };
        self.enrollment.rows = if let Some(review) = &review {
            let id = review.id;
            store.run(move |db| sql::rows(db, &key, id, after)).await?
        } else {
            vec![]
        };
        self.enrollment.local = if review
            .as_ref()
            .is_some_and(|r| matches!(r.phase.as_str(), "applying" | "settings" | "complete"))
        {
            Some(store.run(|db| sql::local(db)).await?)
        } else {
            None
        };
        self.enrollment.review = review;
        self.enrollment.after = after;
        self.observe(error).await
    }
    pub async fn close(self) -> Result<()> {
        self.catalog.close().await.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replaced_and_disconnected_grants_cannot_continue_old_discovery() {
        let mut prefs = Preferences::default();
        prefs.google_grant.id = "fixture-grant".into();
        prefs.google_grant.access.known = true;
        prefs.google_grant.access.drive = true;
        prefs.google_connection_id = "drive:fixture".into();
        let grant = Grant::from_preferences(&prefs);
        grant.check(&prefs).unwrap();
        for variant in 0..6 {
            let mut changed = prefs.clone();
            match variant {
                0 => changed.google_grant.id.push('2'),
                1 => changed.google_lifecycle.revision += 1,
                2 => changed.google_lifecycle.disconnected = true,
                3 => changed.google_connection_id.push('2'),
                4 => changed.google_grant.access.drive = false,
                _ => changed.google_grant.client_id = "other-client".into(),
            }
            assert!(grant.check(&changed).is_err());
        }
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn desktop_discovers_real_records_with_bounded_pages_and_durable_retry_reopen() {
        use super::super::fixture::{Fixture, NAMESPACE};
        use shep_profile_core::drive::catalog::Phase;
        let fixture = Fixture::start(51, true, std::time::Duration::ZERO)
            .await
            .unwrap();
        let drive = fixture
            .connect(NAMESPACE.into(), "drive:fixture")
            .await
            .unwrap();
        let root = fixture.root.path().join("catalogs");
        let grant = Grant {
            principal: "drive:fixture".into(),
            allowed: true,
            ..Default::default()
        };
        let mut session = Session::open(root.clone(), Uuid::new_v4(), grant.clone(), &drive)
            .await
            .unwrap();
        let mut failed = false;
        for _ in 0..1200 {
            let observed = session.run(Action::Advance, Some(&drive)).await.unwrap();
            assert!(observed.rows.len() <= 50);
            let state = observed.state.unwrap();
            if observed.error.is_some() {
                assert!(!failed);
                assert!(state.error.is_some());
                failed = true;
                // A new desktop owner must resume the saved failure and scan.
                session.close().await.unwrap();
                session = Session::open(root.clone(), Uuid::new_v4(), grant.clone(), &drive)
                    .await
                    .unwrap();
                assert!(
                    session
                        .observe(None)
                        .await
                        .unwrap()
                        .state
                        .unwrap()
                        .error
                        .is_some()
                );
                session
                    .run(
                        Action::Retry {
                            revision: state.revision,
                        },
                        None,
                    )
                    .await
                    .unwrap();
            } else if state.phase == Phase::Complete {
                break;
            }
        }
        assert!(failed);
        let first = session.observe(None).await.unwrap();
        assert_eq!(first.state.as_ref().unwrap().phase, Phase::Complete);
        assert_eq!(first.state.as_ref().unwrap().profiles, 51);
        assert_eq!(first.rows.len(), 50);
        assert!(
            first
                .rows
                .iter()
                .all(|p| p.initialized && p.settings == 1 && p.conflicts == 0)
        );
        let after = first.rows.last().unwrap().cursor();
        let last = session
            .run(
                Action::Page {
                    after: Some(after.clone()),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(last.rows.len(), 1);
        assert!(!first.rows.iter().any(|p| p.profile == last.rows[0].profile));
        let invalid = session
            .run(
                Action::Page {
                    after: Some("x".repeat(74)),
                },
                None,
            )
            .await
            .unwrap();
        assert!(invalid.error.is_some());
        assert_eq!(invalid.after, Some(after));
        assert_eq!(invalid.rows, last.rows);
        let revision = first.state.unwrap().revision;
        session.close().await.unwrap();
        let reopened = Session::open(root, Uuid::new_v4(), grant, &drive)
            .await
            .unwrap();
        assert_eq!(
            reopened
                .observe(None)
                .await
                .unwrap()
                .state
                .unwrap()
                .revision,
            revision
        );
        reopened.close().await.unwrap();
        assert!(
            fixture
                .connect(NAMESPACE.into(), "drive:other-fixture")
                .await
                .is_err()
        );
    }
}
