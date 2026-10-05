//! Production mail dispatch with an explicitly owned test workspace and keychain.
use super::*;
use crate::credentials::Credentials;

/// The caller owns the isolated store and supplies a memory credential backend.
/// Profile selection and Google startup are excluded from mail-server scenarios.
pub(crate) fn subscription(store: Store, credentials: Credentials) -> impl Stream<Item = Event> {
    iced::stream::channel(CHANNEL_CAPACITY, move |mut output: Output| async move {
        let (commands, input) = CommandSender::channel();
        let credentials = credentials.with_account_store(store.clone());
        if let Err(error) = store.interrupt_account_setups().await {
            let _ = output
                .send(Event::Error(format!(
                    "Could not recover account setup: {error:#}"
                )))
                .await;
            return;
        }
        let engine = Engine {
            profiles: None,
            credentials: credentials.clone(),
            account_tester: account_setup::tester(false),
            store,
            google: providers::google::Google::with_credentials(credentials.clone()),
            demo: false,
            account_work: Default::default(),
            calendar_work: Default::default(),
            calendar_setup: Default::default(),
            account_setup_writes: Default::default(),
            connection_lifecycle: Default::default(),
            secret_remover: removals::secret_remover(false, credentials.clone()),
            outbound: Arc::new(providers::outgoing::Servers {
                credentials: credentials.clone(),
            }),
            move_connections: Arc::new(providers::mail::moves::ImapMoveConnections {
                credentials: credentials.clone(),
            }),
            google_connection_lock: Default::default(),
            passphrases: Arc::new(backup::OsPassphraseStore(credentials.clone())),
            restore_credentials: Arc::new(backup::restore::OsCredentialRestorer(credentials)),
            backup_uploads: Default::default(),
            mail_sync_settings: Default::default(),
            provider_slots: Default::default(),
            printing: Default::default(),
            bulk_control: Default::default(),
        };
        let workspace = match engine.store.workspace().await {
            Ok(workspace) => workspace,
            Err(error) => {
                let _ = output.send(Event::Error(error.to_string())).await;
                return;
            }
        };
        engine
            .mail_sync_settings
            .set(workspace.preferences.mail_check_seconds);
        for command in [
            Command::IndexConversations,
            Command::CleanupCredentials,
            Command::RepairOutgoing,
        ] {
            if commands.try_send(command).is_err() {
                let _ = output
                    .send(Event::Error("Could not queue mail test startup.".into()))
                    .await;
                return;
            }
        }
        if output
            .send(Event::Ready(commands, Arc::new(workspace), false))
            .await
            .is_err()
        {
            return;
        }
        if let Err(error) = engine.store.recover_creations().await {
            let _ = output
                .send(Event::Error(format!(
                    "Could not recover folder creation requests. {error:#}"
                )))
                .await;
            return;
        }
        if let Err(error) = engine.store.recover_calendar_actions().await {
            let _ = output
                .send(Event::Error(format!(
                    "Could not recover calendar changes. {error:#}"
                )))
                .await;
            return;
        }
        if let Err(error) = engine.send_calendar(&mut output).await {
            let _ = output.send(Event::Error(error.to_string())).await;
            return;
        }
        engine.run(input, output).await;
    })
}
