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
        mut output: Output,
        mut stop: account_work::Stop,
    ) -> anyhow::Result<()> {
        let setup = async {
            self.store.ensure_folder_idle(account.id.clone()).await?;
            let account = self.account(&account.id).await?;
            let password = self.credentials.read(&account.id).await?;
            let known = self.store.known(account.id.clone()).await?;
            Ok::<_, anyhow::Error>((account, password, known))
        };
        let (account, password, known) = tokio::select! {
            biased;
            _ = stop.cancelled() => return Ok(()),
            result = setup => result?,
        };
        let store = self.store.clone();
        let receive = move |mut rx: mpsc::Receiver<MailSyncItem>| async move {
            let mut last = Instant::now();
            let mut skipped = 0;
            while let Some(mail) = rx.recv().await {
                if matches!(mail, MailSyncItem::SkippedLarge) {
                    skipped += 1;
                }
                let folders_changed = matches!(&mail, MailSyncItem::Folders(..));
                match mail {
                    MailSyncItem::Message(mail) => {
                        if let Some(arrival) = store.sync_message(mail).await? {
                            output.send(Event::MailArrived(Arc::new(arrival))).await?;
                        }
                    }
                    mail => store.apply_sync(mail).await?,
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
            if skipped > 0 {
                output
                    .send(Event::Notice(format!(
                        "Skipped {skipped} messages larger than the 25 MiB download limit."
                    )))
                    .await?;
            }
            Ok::<_, anyhow::Error>(())
        };
        let provider = providers::mail::provider(account.protocol);
        let Some(folders) = download(
            stop,
            |tx| provider.sync(&account, &password, &known, tx),
            receive,
        )
        .await?
        else {
            // A waiting write interrupted the read-only download. Already
            // received items were committed; a later check resumes discovery.
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
    let (tx, rx) = mpsc::channel(8);
    let fetch = async move {
        tokio::select! {
            biased;
            _ = stop.cancelled() => Ok(None),
            result = tokio::time::timeout(Duration::from_secs(480), provider(tx)) => {
                result.context("Account sync timed out")?.map(Some)
            },
        }
    };
    // try_join! would drop the receiver on provider error, detaching an active
    // SQLite spawn_blocking write and releasing the account too early.
    let (fetched, received) = tokio::join!(fetch, receive(rx));
    received?;
    fetched
}

#[cfg(test)]
mod tests;
