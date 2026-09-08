use super::*;
use crate::providers::calendar::{CalDav, discovery::DiscoveredCalendar};
use secrecy::ExposeSecret;

impl Engine {
    pub(super) async fn discover_calendars(
        &self,
        url: String,
        username: String,
        password: SecretString,
    ) -> anyhow::Result<Vec<DiscoveredCalendar>> {
        if self.demo {
            #[cfg(feature = "test-support")]
            return crate::test_support::discover_calendars(
                &url,
                &username,
                password.expose_secret(),
            );
            #[cfg(not(feature = "test-support"))]
            anyhow::bail!("Calendar connection testing is disabled in preview.");
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(90),
            CalDav {
                http: self.google.http.clone(),
            }
            .discover(&url, &username, password.expose_secret()),
        )
        .await
        .context("Calendar discovery timed out. Check the address and try again.")?
    }

    pub(super) async fn connect_calendars(
        &self,
        mut sources: Vec<CalendarSource>,
        password: SecretString,
        observed_revision: u64,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !sources.is_empty() && sources.len() <= 256,
            "Choose at least one calendar to connect."
        );
        let mut seen = std::collections::HashSet::new();
        for source in &mut sources {
            anyhow::ensure!(
                source.kind == CalendarKind::CalDav
                    && !source.name.trim().is_empty()
                    && !source.username.trim().is_empty(),
                "Enter a calendar name and username."
            );
            source.url = providers::calendar::validate_caldav_url(&source.url)?.to_string();
            anyhow::ensure!(
                seen.insert((source.url.clone(), source.username.clone())),
                "The same calendar was selected more than once."
            );
        }
        // A separate lock namespace avoids conflicts with externally supplied
        // legacy source identifiers. Network discovery happens before this lock.
        let _lifecycle = self.connection_lifecycle_lock.lock().await;
        let _setup = self.calendar_setup_lock.lock().await;
        let mut guards = Vec::new();
        let existing: Vec<CalendarSource> = self.store.get("calendars").await?;
        for source in &mut sources {
            if let Some(old) = existing.iter().find(|s| {
                s.kind == CalendarKind::CalDav
                    && providers::calendar::validate_caldav_url(&s.url)
                        .is_ok_and(|url| url.as_str() == source.url)
                    && s.username == source.username
            }) {
                source.id = old.id.clone();
            } else {
                // Never accept a caller-supplied OS credential owner identifier.
                use sha2::{Digest, Sha256};
                source.id = format!(
                    "caldav:{:x}",
                    Sha256::digest(format!("{}\0{}", source.username, source.url).as_bytes())
                );
            }
        }
        let mut ids: Vec<_> = sources.iter().map(|s| s.id.clone()).collect();
        ids.sort();
        ids.dedup();
        anyhow::ensure!(
            ids.len() == sources.len(),
            "The saved calendar identities conflict. Check the connected calendars before retrying."
        );
        for id in ids {
            guards.push(self.calendar_access(&id).await);
            self.store
                .check_calendar_reconnect(id, observed_revision)
                .await?;
        }
        if !self.demo {
            anyhow::ensure!(
                !password.expose_secret().is_empty(),
                "Enter your calendar password."
            );
            for source in &sources {
                providers::write_secret(&source.id, password.clone()).await.context("Could not save a calendar password. Unlock your credential store and retry Connect.")?;
            }
        }
        self.store
            .save_sources(sources)
            .await
            .context("Could not finish saving the calendar connections. Retry Connect to finish.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn calendar_connect_is_idempotent_and_never_uses_a_supplied_secret_alias() {
        let engine = super::super::calendar_tests::engine();
        let source = CalendarSource {
            id: "google-oauth".into(),
            name: "Home".into(),
            kind: CalendarKind::CalDav,
            url: "https://calendar.example.test/home/".into(),
            username: "fixture".into(),
            access: CalendarAccess::READ_ONLY,
        };
        engine
            .connect_calendars(vec![source.clone()], "fixture".into(), 0)
            .await
            .unwrap();
        let mut reconnected = source;
        reconnected.url = "https://CALENDAR.example.test/home/".into();
        engine
            .connect_calendars(vec![reconnected], "fixture".into(), 0)
            .await
            .unwrap();
        let sources: Vec<CalendarSource> = engine.store.get("calendars").await.unwrap();
        assert_eq!(sources.len(), 1);
        assert!(sources[0].id.starts_with("caldav:"));
        assert!(sources[0].access.read_only());
        let mut invalid = sources[0].clone();
        invalid.url = "http://external.example".into();
        assert!(
            engine
                .connect_calendars(vec![sources[0].clone(), invalid], "fixture".into(), 0)
                .await
                .is_err()
        );
        assert_eq!(
            engine
                .store
                .get::<Vec<CalendarSource>>("calendars")
                .await
                .unwrap(),
            sources
        );
    }
    #[tokio::test]
    async fn read_only_calendar_mutations_are_rejected_by_engine() {
        let engine = super::super::calendar_tests::engine();
        let event = super::super::calendar_tests::event("home");
        engine
            .store
            .save_source(CalendarSource {
                id: "home".into(),
                name: "Shared".into(),
                kind: CalendarKind::CalDav,
                url: "https://calendar.example.test/home/".into(),
                username: "fixture".into(),
                access: CalendarAccess::READ_ONLY,
            })
            .await
            .unwrap();
        let (output, _) = futures::channel::mpsc::channel(32);
        assert!(
            engine
                .execute(Command::SaveEvent(event.clone()), output.clone())
                .await
                .is_err()
        );
        assert!(
            engine
                .execute(Command::DeleteEvent(event), output)
                .await
                .is_err()
        );
        assert!(engine.store.events().await.unwrap().is_empty());
    }
}
