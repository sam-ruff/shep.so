//! Times cache operations against a copy of a real workspace database.
//! Run with `SHEP_PROFILE_DB=/path/to/copy.sqlite cargo test --release
//! --test real_db_profile -- --ignored --nocapture`. It writes to the copy.
use shep::{model::*, store::Store};
use std::time::Instant;

async fn timed<T>(label: &str, work: impl std::future::Future<Output = anyhow::Result<T>>) -> T {
    let started = Instant::now();
    let result = work.await;
    println!(
        "{:>9.1} ms  {label}",
        started.elapsed().as_secs_f64() * 1000.
    );
    match result {
        Ok(value) => value,
        Err(error) => panic!("{label}: {error:#}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs SHEP_PROFILE_DB pointing at a copied workspace database"]
async fn profile_cache_operations() {
    let Ok(path) = std::env::var("SHEP_PROFILE_DB") else {
        panic!("Set SHEP_PROFILE_DB to a copied shep.sqlite");
    };
    let store = timed("open", async { Store::open(&path) }).await;
    let workspace = timed("workspace", store.workspace()).await;
    let page = timed("query inbox", store.query(MailQuery::default())).await;
    for account in &workspace.accounts {
        let known = timed(
            &format!("known {}", account.name),
            store.known(account.id.clone()),
        )
        .await;
        println!("           {} known ids", known.len());
        timed("folder_modseqs", store.folder_modseqs(account.id.clone())).await;
        timed(
            "ensure_folder_idle",
            store.ensure_folder_idle(account.id.clone()),
        )
        .await;
    }
    timed("activity", store.activity()).await;
    if let Some(first) = page.rows.first() {
        timed("detail", store.detail(first.id.clone())).await;
        timed("conversation", store.conversation(first.id.clone(), None)).await;
    }
    let ids: Vec<(String, bool, bool)> = store
        .run(|c| {
            Ok(
                c.prepare("SELECT id,unread,starred FROM messages WHERE folder='Archive'")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                    .collect::<Result<Vec<_>, _>>()?,
            )
        })
        .await
        .unwrap_or_default();
    let epoch = timed("sync_epoch", store.sync_epoch()).await;
    let count = ids.len();
    timed(
        &format!("unchanged flags for {count} messages"),
        store.apply_sync_since(MailSyncItem::Flags(ids), Some(epoch)),
    )
    .await;
    for _ in 0..3 {
        timed("workspace again", store.workspace()).await;
        timed("query inbox again", store.query(MailQuery::default())).await;
    }
}
