use super::*;
use crate::profile_sync::drive::tests::{binding, reserved};

fn records(count: usize) -> Vec<RemoteRecord> {
    (0..count)
        .map(|_| {
            let mut record = reserved().remote;
            record.id = Uuid::new_v4().to_string();
            record.key.operation = Uuid::new_v4();
            record
        })
        .collect()
}
fn page(scan: &Scan, records: Vec<RemoteRecord>, next: Option<&str>) -> Page {
    Page {
        binding: scan.binding.clone(),
        cursor: scan.cursor.clone(),
        profile: scan.profile,
        records,
        next: next.map(str::to_owned),
    }
}

#[tokio::test]
async fn profile_discovery_resumes_across_restart_and_reads_only_completed_bounded_pages() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("drive.sqlite");
    let journal = Journal::open(Some(&path)).unwrap();
    let scan = journal.begin_scan(binding(), None).await.unwrap();
    assert!(!scan.complete());
    assert!(journal.scan_entries(&scan, None).await.is_err());
    let scan = journal
        .append_page(&scan, page(&scan, vec![], Some("first-data")))
        .await
        .unwrap();
    let all = records(125);
    let scan = journal
        .append_page(&scan, page(&scan, all[..100].to_vec(), Some("last")))
        .await
        .unwrap();
    assert_eq!(scan.files(), 100);
    assert!(journal.scan_entries(&scan, None).await.is_err());
    drop(journal);
    let journal = Journal::open(Some(&path)).unwrap();
    let saved = journal
        .resume_scan(&binding(), None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved, scan);
    let completed = journal
        .append_page(&saved, page(&saved, all[100..].to_vec(), None))
        .await
        .unwrap();
    assert!(completed.complete());
    assert_eq!(completed.files(), 125);
    let first = journal.scan_entries(&completed, None).await.unwrap();
    let second = journal
        .scan_entries(&completed, Some(first.last().unwrap().position))
        .await
        .unwrap();
    let third = journal
        .scan_entries(&completed, Some(second.last().unwrap().position))
        .await
        .unwrap();
    assert_eq!((first.len(), second.len(), third.len()), (50, 50, 25));
    let actual = first
        .into_iter()
        .chain(second)
        .chain(third)
        .map(|e| e.record)
        .collect::<Vec<_>>();
    assert_eq!(actual, all);
}

#[tokio::test]
async fn profile_discovery_rejects_cross_page_loops_duplicates_and_late_results_atomically() {
    let journal = Journal::open(None).unwrap();
    let scan = journal.begin_scan(binding(), None).await.unwrap();
    let all = records(3);
    let scan = journal
        .append_page(&scan, page(&scan, vec![all[0].clone()], Some("a")))
        .await
        .unwrap();
    let scan = journal
        .append_page(&scan, page(&scan, vec![all[1].clone()], Some("b")))
        .await
        .unwrap();
    let mut duplicate_operation = all[0].clone();
    duplicate_operation.id = "another-drive-id".into();
    for bad in [
        page(&scan, vec![all[2].clone()], Some("a")),
        page(&scan, vec![all[0].clone()], None),
        page(&scan, vec![duplicate_operation], None),
    ] {
        assert!(journal.append_page(&scan, bad).await.is_err());
        assert_eq!(
            journal
                .resume_scan(&binding(), None)
                .await
                .unwrap()
                .unwrap(),
            scan
        );
    }
    let done = journal
        .append_page(&scan, page(&scan, vec![all[2].clone()], None))
        .await
        .unwrap();
    assert_eq!(done.files(), 3);
    assert!(
        journal
            .append_page(&scan, page(&scan, vec![], None))
            .await
            .is_err()
    );
    let next = journal.begin_scan(binding(), None).await.unwrap();
    assert!(journal.scan_entries(&done, None).await.is_err());
    assert!(
        journal
            .append_page(&scan, page(&scan, vec![], None))
            .await
            .is_err()
    );
    let mut wrong = page(&next, vec![], None);
    wrong.binding = Binding::new("drive:other-user".into(), "so.shep.fixture".into()).unwrap();
    assert!(journal.append_page(&next, wrong).await.is_err());
    assert_eq!(
        journal
            .resume_scan(&binding(), None)
            .await
            .unwrap()
            .unwrap(),
        next
    );
}

#[tokio::test]
async fn profile_discovery_empty_is_explicit_and_scan_replacement_keeps_uploads() {
    let journal = Journal::open(None).unwrap();
    let upload = reserved();
    journal.prepare(upload.clone()).await.unwrap();
    let mut scan = journal.begin_scan(binding(), None).await.unwrap();
    // More than the backup list's 100-page ceiling: profile history is paged
    // without imposing that unrelated whole-list limit.
    for page_number in 0..102 {
        scan = journal
            .append_page(
                &scan,
                page(&scan, vec![], Some(&format!("page-{page_number}"))),
            )
            .await
            .unwrap();
        assert!(!scan.complete());
    }
    scan = journal
        .append_page(&scan, page(&scan, vec![], None))
        .await
        .unwrap();
    assert!(scan.complete());
    assert!(journal.scan_entries(&scan, None).await.unwrap().is_empty());
    let only = journal
        .begin_scan(
            binding(),
            Some((upload.remote.key.profile, upload.remote.key.generation)),
        )
        .await
        .unwrap();
    assert!(
        journal
            .resume_scan(&binding(), None)
            .await
            .unwrap()
            .unwrap()
            .complete()
    );
    let mut wrong = reserved().remote;
    wrong.key.profile = Uuid::new_v4();
    assert!(
        journal
            .append_page(&only, page(&only, vec![wrong], None))
            .await
            .is_err()
    );
    journal.begin_scan(binding(), None).await.unwrap();
    assert_eq!(
        journal
            .load(&binding(), upload.remote.key)
            .await
            .unwrap()
            .unwrap()
            .upload
            .remote,
        upload.remote
    );
}
