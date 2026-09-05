//! Explicit opt-in, read-only diagnostics against a user's saved account.
//! These ignored tests never send email, move messages, or change flags.
use shep::{model::Account, providers, store::Store};

#[tokio::test]
#[ignore = "Requires SHEP_LIVE_ACCOUNT_ID and the current user's unlocked OS keychain"]
async fn saved_account_connections() {
    let id = std::env::var("SHEP_LIVE_ACCOUNT_ID").expect("Set the saved account ID explicitly");
    let directory = directories::ProjectDirs::from("so", "shep", "Shep").unwrap();
    let store = Store::open(directory.data_local_dir().join("shep.sqlite")).unwrap();
    let accounts: Vec<Account> = store.get("accounts").await.unwrap();
    let account = accounts
        .into_iter()
        .find(|a| a.id == id)
        .expect("Saved account not found");
    let password = providers::read_secret(&account.id)
        .await
        .expect("Could not read the saved account credential");
    println!("Saved credential is accessible; probing incoming connection (no mutations)");
    let result = providers::mail::test_incoming(&account, &password).await;
    match result {
        Ok(message) => println!("{message}"),
        Err(error) => panic!("Incoming connection failed: {error:#}"),
    }
    let smtp = providers::mail::test_smtp(&account, &password)
        .await
        .expect("SMTP handshake/authentication failed");
    println!("{smtp}");
}

#[cfg(feature = "test-support")]
#[tokio::test]
#[ignore = "Opt-in: downloads saved account mail into the current user's local cache; remote mail is unchanged"]
async fn saved_account_inbox_sync_to_local_cache() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("shep=info")
        .try_init();
    let id = std::env::var("SHEP_LIVE_ACCOUNT_ID").expect("Set the saved account ID explicitly");
    let directory = directories::ProjectDirs::from("so", "shep", "Shep").unwrap();
    let store = Store::open(directory.data_local_dir().join("shep.sqlite")).unwrap();
    let accounts: Vec<Account> = store.get("accounts").await.unwrap();
    let account = accounts
        .into_iter()
        .find(|a| a.id == id)
        .expect("Saved account not found");
    let password = providers::read_secret(&account.id).await.unwrap();
    let known = store.known(account.id.clone()).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let receive = async {
        let mut messages = 0;
        while let Some(item) = rx.recv().await {
            if matches!(item, shep::model::MailSyncItem::Message(_)) {
                messages += 1;
            }
            store.apply_sync(item).await?;
        }
        Ok::<_, anyhow::Error>(messages)
    };
    let download = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        providers::mail::sync_inbox(&account, &password, &known, tx),
    );
    let (result, count) = tokio::join!(download, receive);
    let count = count.expect("Cache write failed");
    match result {
        Ok(Ok(folders)) => println!(
            "Sync succeeded: {count} messages downloaded (folder listing: {} folders; only Inbox downloaded).",
            folders.len()
        ),
        Ok(Err(error)) => panic!("Sync failed after {count} messages: {error:#}"),
        Err(_) => panic!("Sync timed out after {count} messages"),
    }
}
