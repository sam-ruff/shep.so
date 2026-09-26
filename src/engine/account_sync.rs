use super::*;

impl Engine {
    pub(super) async fn sync_account(
        &self,
        account: Account,
        output: Output,
    ) -> anyhow::Result<()> {
        let engine = self.clone();
        self.account_work
            .sync(&account.id.clone(), move |stop| async move {
                engine.download_account(account, output, stop).await
            })
            .await
    }

    #[cfg(feature = "test-support")]
    pub(super) async fn preview_held_account_sync(&self, mut output: Output) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.demo,
            "Held sync is available only in the isolated preview"
        );
        let account = self
            .store
            .query(MailQuery::default())
            .await?
            .rows
            .first()
            .context("Fixture inbox is missing")?
            .account_id
            .clone();
        let store = self.store.clone();
        self.account_work
            .sync(&account, |stop| async move {
                output.send(Event::PreviewAccountSync(true)).await?;
                download(
                    stop,
                    |tx| async move {
                        tx.send(MailSyncItem::Flags(vec![])).await?;
                        std::future::pending::<anyhow::Result<Vec<String>>>().await
                    },
                    |mut rx| async move {
                        while let Some(item) = rx.recv().await {
                            store.apply_sync(item).await?;
                        }
                        Ok(())
                    },
                )
                .await?;
                output.send(Event::PreviewAccountSync(false)).await?;
                Ok(())
            })
            .await
    }

    async fn download_account(
        &self,
        account: Account,
        output: Output,
        stop: account_work::Stop,
    ) -> anyhow::Result<()> {
        // Taken before the first SELECT: every listing this check applies is
        // at or after it, so the ledger can rank later local writes above them.
        let epoch = self.store.sync_epoch().await?;
        let id = account.id.clone();
        let result = self.download_since(account, output, stop, epoch).await;
        self.store.sync_finished(id, epoch).await?;
        result
    }

    async fn download_since(
        &self,
        account: Account,
        mut output: Output,
        mut stop: account_work::Stop,
        epoch: crate::store::SyncEpoch,
    ) -> anyhow::Result<()> {
        let setup = async {
            self.store.ensure_folder_idle(account.id.clone()).await?;
            let account = self.account(&account.id).await?;
            let password = self.credentials.account_password(&account, false).await?;
            let known = self.store.known(account.id.clone()).await?;
            let resume = self.store.folder_modseqs(account.id.clone()).await?;
            Ok::<_, anyhow::Error>((account, password, known, resume))
        };
        let (account, password, known, resume) = tokio::select! {
            biased;
            _ = stop.cancelled() => return Ok(()),
            result = setup => result?,
        };
        let store = self.store.clone();
        let receive = move |mut rx: mpsc::Receiver<MailSyncItem>| async move {
            let mut last = Instant::now();
            while let Some(mail) = rx.recv().await {
                let folders_changed = matches!(&mail, MailSyncItem::Folders(..));
                match mail {
                    MailSyncItem::Message(mail) => {
                        if let Some(arrival) = store.sync_message_since(mail, Some(epoch)).await? {
                            output.send(Event::MailArrived(Arc::new(arrival))).await?;
                        }
                    }
                    MailSyncItem::StagedMessage(mail) => {
                        if let Some(arrival) =
                            store.sync_staged_message_since(mail, Some(epoch)).await?
                        {
                            output.send(Event::MailArrived(Arc::new(arrival))).await?;
                        }
                    }
                    mail => store.apply_sync_since(mail, Some(epoch)).await?,
                }
                if folders_changed {
                    output
                        .send(Event::Workspace(Arc::new(store.workspace().await?)))
                        .await?;
                }
                if last.elapsed() > Duration::from_millis(250) {
                    output.send(Event::Changed).await?;
                    last = Instant::now();
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        let provider = providers::mail::provider(account.protocol);
        let Some(folders) = download(
            stop,
            |tx| {
                provider.sync_resuming(
                    &account,
                    &password,
                    &known,
                    &resume,
                    tx,
                    self.store.connection_key().is_none(),
                )
            },
            receive,
        )
        .await?
        else {
            // Exclusive account work or shutdown interrupted the read-only
            // download. Already received items were committed; a later check
            // resumes discovery.
            return Ok(());
        };
        self.store
            .run(move |c| {
                use rusqlite::OptionalExtension;
                let old: Option<String> = c
                    .query_row("SELECT value FROM kv WHERE key='folders'", [], |r| r.get(0))
                    .optional()?;
                let mut all: Vec<String> = old
                    .map(|s| serde_json::from_str(&s))
                    .transpose()?
                    .unwrap_or_default();
                for f in folders {
                    if !all.contains(&f) {
                        all.push(f);
                    }
                }
                c.execute(
                    "INSERT OR REPLACE INTO kv VALUES('folders',?)",
                    [serde_json::to_string(&all)?],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}

/// Always finish observing cache writes, including after provider error,
/// timeout, interruption or cancellation of the containing refresh cycle.
async fn download<P, PF, R, RF>(
    mut stop: account_work::Stop,
    provider: P,
    receive: R,
) -> anyhow::Result<Option<Vec<String>>>
where
    P: FnOnce(mpsc::Sender<MailSyncItem>) -> PF,
    PF: std::future::Future<Output = anyhow::Result<Vec<String>>>,
    R: FnOnce(mpsc::Receiver<MailSyncItem>) -> RF,
    RF: std::future::Future<Output = anyhow::Result<()>>,
{
    let (tx, mut rx) = mpsc::channel(8);
    let (forward, messages) = mpsc::channel(8);
    let (progress, mut changed) = tokio::sync::watch::channel(());
    let relay = async move {
        while let Some(item) = rx.recv().await {
            progress.send_replace(());
            if matches!(item, MailSyncItem::DownloadProgress) {
                continue;
            }
            if forward.send(item).await.is_err() {
                break;
            }
        }
    };
    let fetch = async move {
        let result = provider(tx);
        tokio::pin!(result);
        let mut progress_open = true;
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break Ok(None),
                result = &mut result => break result.map(Some),
                _ = tokio::time::sleep(Duration::from_secs(480)) => break Err(anyhow::anyhow!("Account sync timed out without download progress")),
                result = changed.changed(), if progress_open => { progress_open = result.is_ok(); },
            }
        }
    };
    // try_join! would drop the receiver on provider error, detaching an active
    // SQLite spawn_blocking write and releasing the account too early.
    let (fetched, received, ()) = tokio::join!(fetch, receive(messages), relay);
    received?;
    fetched
}

#[cfg(test)]
mod tests;
