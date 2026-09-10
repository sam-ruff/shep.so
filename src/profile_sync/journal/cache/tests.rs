use super::*;
use crate::profile_sync::drive::{
    Page,
    tests::{binding, reserved},
};

async fn complete(journal: &Journal, remote: Option<RemoteRecord>) -> Scan {
    let scan = journal.begin_scan(binding(), None).await.unwrap();
    journal
        .append_page(
            &scan,
            Page {
                binding: binding(),
                cursor: None,
                profile: None,
                records: remote.into_iter().collect(),
                next: None,
            },
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn profile_download_cache_reopens_exact_verified_bytes_and_requires_current_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drive.sqlite");
    let journal = Journal::open(Some(&path)).unwrap();
    let original = reserved();
    let scan = complete(&journal, Some(original.remote.clone())).await;
    assert!(
        journal
            .cached_download(&scan, &original.remote)
            .await
            .unwrap()
            .is_none()
    );
    journal
        .cache_download(&scan, &original.remote, original.record.clone())
        .await
        .unwrap();
    drop(journal);
    let journal = Journal::open(Some(&path)).unwrap();
    assert_eq!(
        journal
            .cached_download(&scan, &original.remote)
            .await
            .unwrap()
            .unwrap()
            .bytes(),
        original.record.bytes()
    );
    let fresh = complete(&journal, Some(original.remote.clone())).await;
    assert!(
        journal
            .cached_download(&scan, &original.remote)
            .await
            .is_err()
    );
    assert!(
        journal
            .cached_download(&fresh, &original.remote)
            .await
            .unwrap()
            .is_some()
    );
    let missing = complete(&journal, None).await;
    assert!(
        journal
            .cached_download(&missing, &original.remote)
            .await
            .is_err()
    );
    assert!(
        journal
            .cache_download(&fresh, &original.remote, original.record)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn profile_download_cache_bounds_corruption_and_only_accepts_verified_repair() {
    let journal = Journal::open(None).unwrap();
    let original = reserved();
    let scan = complete(&journal, Some(original.remote.clone())).await;
    journal
        .cache_download(&scan, &original.remote, original.record.clone())
        .await
        .unwrap();
    for size in [original.record.bytes().len(), MAX_RECORD_BYTES + 1] {
        journal
            .worker
            .run(move |c| {
                c.execute("UPDATE downloads SET bytes=zeroblob(?)", [size as i64])?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            journal
                .cached_download(&scan, &original.remote)
                .await
                .unwrap()
                .is_none()
        );
        journal
            .cache_download(&scan, &original.remote, original.record.clone())
            .await
            .unwrap();
        assert_eq!(
            journal
                .cached_download(&scan, &original.remote)
                .await
                .unwrap()
                .unwrap()
                .bytes(),
            original.record.bytes()
        );
    }
    let mut operation = original.record.operation().clone();
    operation.operation = Uuid::new_v4();
    let wrong = Record::decode(binding().namespace(), operation.encode().unwrap()).unwrap();
    assert!(
        journal
            .cache_download(&scan, &original.remote, wrong)
            .await
            .is_err()
    );
    assert_eq!(
        journal
            .cached_download(&scan, &original.remote)
            .await
            .unwrap()
            .unwrap()
            .bytes(),
        original.record.bytes()
    );
}

#[tokio::test]
async fn profile_download_cache_does_not_reuse_replaced_file_metadata_or_partial_lists() {
    let journal = Journal::open(None).unwrap();
    let original = reserved();
    let scan = complete(&journal, Some(original.remote.clone())).await;
    journal
        .cache_download(&scan, &original.remote, original.record.clone())
        .await
        .unwrap();
    let mut replaced = original.remote.clone();
    replaced.id = "another-file-id".into();
    let scan = complete(&journal, Some(replaced.clone())).await;
    assert!(
        journal
            .cached_download(&scan, &replaced)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        journal
            .cached_download(&scan, &original.remote)
            .await
            .is_err()
    );
    let partial = journal.begin_scan(binding(), None).await.unwrap();
    assert!(journal.cached_download(&partial, &replaced).await.is_err());
    // An unrelated Drive identity cannot expose these bytes either.
    let mut other = binding();
    other.identity = "drive:another-user".into();
    assert!(
        !journal
            .fixture_cached_download(&other, &original.remote)
            .await
    );
}
