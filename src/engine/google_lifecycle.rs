use super::*;

impl Engine {
    /// Caller holds the Google write guard, excluding all provider requests and
    /// reconnects while the OS applies cleanup. No remote grant is revoked.
    pub(super) async fn cleanup_google_locked(&self) -> anyhow::Result<()> {
        let prefs: Preferences = self.store.get("preferences").await?;
        if !prefs.google_lifecycle.cleanup_pending {
            return Ok(());
        }
        if !self.demo {
            self.google.clear_credentials().await?;
        }
        self.store
            .finish_google_cleanup(prefs.google_lifecycle.revision)
            .await
    }

    pub(super) async fn disconnect_google(
        &self,
        revision: u64,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        let _google = self.google_connection_lock.write().await;
        let _lifecycle = self.connection_lifecycle.write().await;
        self.store.disconnect_google(revision).await?;
        self.workspace(output).await?;
        if let Err(error) = self.cleanup_google_locked().await {
            output.send(Event::Error(format!("{error:#}"))).await?;
        } else {
            output
                .send(Event::Notice(
                    "Google disconnected. Cached calendars and existing backups were kept.".into(),
                ))
                .await?;
        }
        self.workspace(output).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disconnect_waits_for_active_google_work_then_rejects_stale_review() {
        let engine = super::super::calendar_tests::engine();
        let active = engine.google_connection_lock.read().await;
        let (mut output, _rx) = futures::channel::mpsc::channel(32);
        let mut pending_output = output.clone();
        let disconnect = engine.disconnect_google(0, &mut pending_output);
        tokio::pin!(disconnect);
        assert!(futures::poll!(&mut disconnect).is_pending());
        assert!(
            !engine
                .store
                .workspace()
                .await
                .unwrap()
                .preferences
                .google_lifecycle
                .disconnected
        );
        drop(active);
        disconnect.await.unwrap();
        let prefs = engine.store.workspace().await.unwrap().preferences;
        assert!(prefs.google_lifecycle.disconnected);
        assert!(!prefs.google_lifecycle.cleanup_pending);
        assert!(engine.disconnect_google(0, &mut output).await.is_err());
        assert!(
            Engine::check_backup_target(
                &BackupTarget::GoogleDrive {
                    client_id: String::new(),
                    connection_id: String::new()
                },
                &prefs
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn disconnected_google_event_mutations_are_rejected_even_for_preview_commands() {
        let engine = super::super::calendar_tests::engine();
        let source = CalendarSource {
            id: "google:fixture".into(),
            name: "Google fixture".into(),
            kind: CalendarKind::Google,
            url: String::new(),
            username: String::new(),
            access: Default::default(),
        };
        engine.store.save_source(source).await.unwrap();
        let start = chrono::Utc::now();
        let event = CalendarEvent {
            id: "event".into(),
            source_id: "google:fixture".into(),
            title: "Keep original".into(),
            start,
            end: start + chrono::Duration::hours(1),
            location: String::new(),
            description: String::new(),
            all_day: false,
            etag: None,
            remote_url: None,
        };
        engine.store.save_event(event.clone()).await.unwrap();
        let (mut output, _rx) = futures::channel::mpsc::channel(32);
        engine.disconnect_google(0, &mut output).await.unwrap();
        assert!(
            engine
                .execute(Command::DeleteEvent(event.clone()), output.clone())
                .await
                .is_err()
        );
        assert!(
            engine
                .execute(Command::SaveEvent(event), output)
                .await
                .is_err()
        );
        assert_eq!(
            engine.store.calendar_snapshot().await.unwrap().1[0].title,
            "Keep original"
        );
    }
}
