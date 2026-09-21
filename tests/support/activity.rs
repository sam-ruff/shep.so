use super::*;

pub async fn seed(store: &Store) -> anyhow::Result<()> {
    let account = store
        .get::<Vec<Account>>("accounts")
        .await?
        .into_iter()
        .find(|account| account.id == "preview-work")
        .ok_or_else(|| anyhow::anyhow!("Missing fixture account"))?;
    let connection = crate::mail_actions::connection_key(&account);
    store.run(move |c| {
        for index in 0..36 {
            let job = crate::store::CreationJob {
                id: format!("activity-folder-{index:02}"), account: account.id.clone(), connection: connection.clone(),
                parent: None, name: if index == 0 { "Older folder recovery".into() } else { format!("Pending fixture {index}") },
                stage: if index == 0 { crate::store::CreationStage::Uncertain } else { crate::store::CreationStage::Queued },
                target: Some(crate::folders::Mailbox::flat(format!("Fixture/{index}"))), receipt: None,
                provider_acknowledged: false, error: (index == 0).then(|| "The connection ended before its result was recorded.".into()), revision: 1,
            };
            c.execute("INSERT INTO folder_creations(account,connection,request,target,data) VALUES(?,?,?,?,?)", rusqlite::params![job.account,job.connection,format!("fixture-{index}"),"null",serde_json::to_string(&job)?])?;
        }
        Ok(())
    }).await?;
    let mut backup = crate::backup::history::Entry::new(
        crate::backup::BackupTarget::Local("/fixture/retired-backup-destination".into()),
        "Retained fixture backup".into(),
        crate::backup::format::Options::default(),
    );
    backup.outcome = crate::backup::history::Outcome::NeedsReview;
    backup.detail = "An interrupted backup remains owned by its original destination.".into();
    store.write_backup_history(backup).await
}
