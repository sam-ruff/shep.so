use shep::{
    model::*,
    store::{MailSelectionId, SelectionChange, Store},
};

fn message(id: usize, account: &str, folder: &str) -> StoredMail {
    let body = if id == 0 {
        "test".to_owned()
    } else {
        format!("Notes for test {id}")
    };
    let mut m = parse_mail(account, &id.to_string(), folder,
        format!("From: Person {id:03} <sender{id}@example.test>\r\nSubject: Subject {:03}\r\n\r\n{body}", 200-id).into_bytes(),
        id.is_multiple_of(2), id.is_multiple_of(3)).unwrap();
    m.summary.timestamp = id as i64;
    m.summary.attachment_count = usize::from(id.is_multiple_of(5));
    m
}

async fn fixture(store: &Store) {
    let mut messages: Vec<_> = (0..123).map(|id| message(id, "work", "INBOX")).collect();
    messages.extend((123..131).map(|id| message(id, "personal", "INBOX")));
    messages.extend((131..142).map(|id| message(id, "work", "A. Keep")));
    store.upsert(messages).await.unwrap();
}

async fn query_ids(store: &Store, mut query: MailQuery) -> Vec<String> {
    let mut result = Vec::new();
    query.offset = 0;
    loop {
        let page = store.query(query.clone()).await.unwrap();
        result.extend(page.rows.into_iter().map(|m| m.id));
        if result.len() >= page.total {
            return result;
        }
        query.offset += PAGE_SIZE;
    }
}

async fn selected_ids(store: &Store, id: MailSelectionId, revision: u64) -> Vec<String> {
    let mut result = Vec::new();
    let mut after = None;
    loop {
        let page = store.selected_mail_page(id, revision, after).await.unwrap();
        assert!(page.rows.len() <= PAGE_SIZE);
        result.extend(page.rows.into_iter().map(|m| m.mail.id));
        match page.next_after {
            Some(next) => after = Some(next),
            None => return result,
        }
    }
}

#[tokio::test]
async fn full_selection_has_identical_scope_and_order_to_every_inbox_page() {
    let store = Store::memory().unwrap();
    fixture(&store).await;
    for sort in MailSort::SEARCH {
        for search in ["", "test", "tes", "'; DROP TABLE messages; --"] {
            for folders in [
                None,
                Some(vec![]),
                Some(vec![
                    FolderSelection {
                        account: Some("work".into()),
                        folder: "A. Keep".into(),
                        sent_only: false,
                    },
                    FolderSelection {
                        account: Some("personal".into()),
                        folder: "INBOX".into(),
                        sent_only: false,
                    },
                ]),
            ] {
                let query = MailQuery {
                    account: Some("work".into()),
                    folder: "INBOX".into(),
                    sort,
                    search: search.into(),
                    folders,
                    offset: 50,
                    ..Default::default()
                };
                let expected = query_ids(&store, query.clone()).await;
                let id = MailSelectionId::default();
                let snapshot = store
                    .capture_selection(
                        id,
                        0,
                        query,
                        true,
                        expected.iter().take(PAGE_SIZE).cloned().collect(),
                    )
                    .await
                    .unwrap();
                assert_eq!(snapshot.total, expected.len());
                assert_eq!(snapshot.selected, expected.len());
                assert_eq!(snapshot.available, expected.len());
                assert_eq!(snapshot.visible.len(), expected.len().min(PAGE_SIZE));
                assert_eq!(
                    selected_ids(&store, id, 0).await,
                    expected,
                    "{sort:?} {search}"
                );
                if let (Some(first), Some(last)) = (expected.first(), expected.last()) {
                    store
                        .change_selection(
                            id,
                            0,
                            SelectionChange::Range {
                                anchor: first.clone(),
                                target: last.clone(),
                                additive: false,
                            },
                            vec![],
                        )
                        .await
                        .unwrap();
                    assert_eq!(
                        selected_ids(&store, id, 1).await,
                        expected,
                        "Rebased {sort:?} {search}"
                    );
                }
                store.release_selection(id).await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn read_flag_attachment_and_sent_scopes_match_the_mail_list() {
    let store = Store::memory().unwrap();
    fixture(&store).await;
    let sent = message(150, "work", "Sent Items");
    store.upsert(vec![sent]).await.unwrap();
    store
        .run(|c| {
            c.execute(
                "INSERT INTO sent_folders(account,folder) VALUES('work','Sent Items')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    for query in [
        MailQuery {
            unread_only: true,
            starred_only: true,
            ..Default::default()
        },
        MailQuery {
            read_only: true,
            attachments_only: true,
            ..Default::default()
        },
        MailQuery {
            unread_only: true,
            read_only: true,
            ..Default::default()
        },
        MailQuery {
            account: Some("work".into()),
            sent_only: true,
            ..Default::default()
        },
        MailQuery {
            folders: Some(vec![FolderSelection {
                account: Some("work".into()),
                folder: "Sent".into(),
                sent_only: true,
            }]),
            ..Default::default()
        },
    ] {
        let expected = query_ids(&store, query.clone()).await;
        let id = MailSelectionId::default();
        store
            .capture_selection(id, 0, query, true, vec![])
            .await
            .unwrap();
        assert_eq!(selected_ids(&store, id, 0).await, expected);
        store.release_selection(id).await.unwrap();
    }
}

#[tokio::test]
async fn ranges_span_pages_in_both_directions_and_ctrl_ranges_are_additive() {
    let store = Store::memory().unwrap();
    fixture(&store).await;
    let query = MailQuery {
        account: Some("work".into()),
        folder: "INBOX".into(),
        sort: MailSort::Oldest,
        ..Default::default()
    };
    let ids = query_ids(&store, query.clone()).await;
    let id = MailSelectionId::default();
    let empty = store
        .capture_selection(id, 0, query, false, vec![])
        .await
        .unwrap();
    assert_eq!(empty.selected, 0);
    let first = store
        .change_selection(
            id,
            0,
            SelectionChange::Set {
                id: ids[0].clone(),
                selected: true,
                clear_others: false,
            },
            vec![ids[0].clone()],
        )
        .await
        .unwrap();
    assert_eq!(first.selected, 1);
    assert!(first.visible.contains(&ids[0]));
    let range = store
        .change_selection(
            id,
            1,
            SelectionChange::Range {
                anchor: ids[60].clone(),
                target: ids[110].clone(),
                additive: true,
            },
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(range.selected, 52);
    assert_eq!(
        selected_ids(&store, id, 2).await,
        [&ids[..1], &ids[60..111]].concat()
    );
    let range = store
        .change_selection(
            id,
            2,
            SelectionChange::Range {
                anchor: ids[109].clone(),
                target: ids[40].clone(),
                additive: false,
            },
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(range.selected, 70);
    assert_eq!(selected_ids(&store, id, 3).await, ids[40..110]);
    store
        .change_selection(
            id,
            3,
            SelectionChange::Set {
                id: ids[50].clone(),
                selected: false,
                clear_others: false,
            },
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(selected_ids(&store, id, 4).await.len(), 69);
    let only = store
        .change_selection(
            id,
            4,
            SelectionChange::Set {
                id: ids[5].clone(),
                selected: true,
                clear_others: true,
            },
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(only.selected, 1);
    let all = store
        .change_selection(id, 5, SelectionChange::All, vec![])
        .await
        .unwrap();
    assert_eq!(all.selected, 123);
    let clear = store
        .change_selection(id, 6, SelectionChange::Clear, vec![])
        .await
        .unwrap();
    assert_eq!(clear.selected, 0);
    assert!(clear.accounts.is_empty());
}

#[tokio::test]
async fn new_arrivals_do_not_join_a_selection_and_unavailable_messages_are_explicit() {
    let store = Store::memory().unwrap();
    fixture(&store).await;
    let query = MailQuery {
        folder: "INBOX".into(),
        ..Default::default()
    };
    let id = MailSelectionId::default();
    let initial = store
        .capture_selection(id, 0, query.clone(), true, vec![])
        .await
        .unwrap();
    assert_eq!(initial.selected, 131);
    let new = message(160, "work", "INBOX");
    let new_id = new.summary.id.clone();
    store.upsert(vec![new]).await.unwrap();
    store.remove("work:INBOX:3".into()).await.unwrap();
    let state = store
        .selection_snapshot(id, vec![new_id, "work:INBOX:3".into()])
        .await
        .unwrap();
    assert_eq!((state.selected, state.available), (131, 130));
    assert!(state.visible.is_empty());
    assert_eq!(state.accounts["personal"], 8);
    assert_eq!(state.accounts["work"], 122);
    let ids = selected_ids(&store, id, 0).await;
    assert_eq!(ids.len(), 130);
    assert!(!ids.contains(&"work:INBOX:160".to_string()));
    let updated = store
        .capture_selection(id, 1, query, true, vec![])
        .await
        .unwrap();
    assert_eq!((updated.selected, updated.available), (131, 131));
    assert!(
        selected_ids(&store, id, 1)
            .await
            .contains(&"work:INBOX:160".into())
    );
}

#[tokio::test]
async fn stale_changes_and_failed_ranges_cannot_replace_newer_selection_or_a_review() {
    let store = Store::memory().unwrap();
    fixture(&store).await;
    let id = MailSelectionId::default();
    store
        .capture_selection(
            id,
            4,
            MailQuery {
                folder: "INBOX".into(),
                ..Default::default()
            },
            false,
            vec![],
        )
        .await
        .unwrap();
    store
        .change_selection(
            id,
            4,
            SelectionChange::Set {
                id: "work:INBOX:1".into(),
                selected: true,
                clear_others: false,
            },
            vec![],
        )
        .await
        .unwrap();
    assert!(
        store
            .change_selection(id, 4, SelectionChange::Clear, vec![])
            .await
            .is_err()
    );
    assert!(
        store
            .capture_selection(id, 5, MailQuery::default(), true, vec![])
            .await
            .is_err()
    );
    assert!(
        store
            .change_selection(
                id,
                5,
                SelectionChange::Range {
                    anchor: "work:INBOX:1".into(),
                    target: "outside".into(),
                    additive: false
                },
                vec![]
            )
            .await
            .is_err()
    );
    assert!(
        store
            .change_selection(id, 5, SelectionChange::All, vec!["x".into(); PAGE_SIZE + 1])
            .await
            .is_err()
    );
    assert_eq!(
        store.selection_snapshot(id, vec![]).await.unwrap().revision,
        5
    );
    assert_eq!(selected_ids(&store, id, 5).await, vec!["work:INBOX:1"]);
    assert!(store.freeze_selection(id, 4).await.is_err());
    let review = store.freeze_selection(id, 5).await.unwrap();
    assert!(review.frozen);
    assert_eq!(review.selected, 1);
    store
        .change_selection(id, 5, SelectionChange::Clear, vec![])
        .await
        .unwrap();
    assert!(store.selected_mail_page(id, 5, None).await.is_err());
    store.release_selection(id).await.unwrap();
    assert_eq!(
        selected_ids(&store, review.id, 0).await,
        vec!["work:INBOX:1"]
    );
    assert!(
        store
            .change_selection(review.id, 0, SelectionChange::All, vec![])
            .await
            .is_err()
    );
    assert!(
        store
            .capture_selection(review.id, 1, MailQuery::default(), true, vec![])
            .await
            .is_err()
    );
    store.release_selection(review.id).await.unwrap();
    store.release_selection(review.id).await.unwrap();
    assert!(store.selection_snapshot(review.id, vec![]).await.is_err());
}

#[tokio::test]
async fn snapshot_pages_use_current_flags_and_do_not_persist_into_other_connections_or_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    fixture(&store).await;
    let id = MailSelectionId::default();
    store
        .capture_selection(
            id,
            0,
            MailQuery {
                folder: "INBOX".into(),
                sort: MailSort::Oldest,
                ..Default::default()
            },
            true,
            vec![],
        )
        .await
        .unwrap();
    let mut mail = store
        .query(MailQuery {
            folder: "INBOX".into(),
            sort: MailSort::Oldest,
            ..Default::default()
        })
        .await
        .unwrap()
        .rows
        .remove(0);
    mail.unread = false;
    mail.starred = false;
    store.flags(mail.clone()).await.unwrap();
    let page = store.selected_mail_page(id, 0, None).await.unwrap();
    assert_eq!(page.rows[0].mail.id, mail.id);
    assert!(!page.rows[0].mail.unread);
    assert!(!page.rows[0].mail.starred);
    let other = Store::open(&path).unwrap();
    assert!(other.selection_snapshot(id, vec![]).await.is_err());
    assert_eq!(other.query(MailQuery::default()).await.unwrap().total, 142);
    drop(other);
    drop(store);
    let reopened = Store::open(path).unwrap();
    assert!(reopened.selection_snapshot(id, vec![]).await.is_err());
    assert_eq!(
        reopened.query(MailQuery::default()).await.unwrap().total,
        142
    );
}

#[tokio::test]
async fn explicit_arrival_selection_preserves_choices_and_frozen_review() {
    let store = Store::memory().unwrap();
    store
        .upsert(vec![
            message(0, "work", "INBOX"),
            message(2, "work", "INBOX"),
            message(4, "work", "INBOX"),
        ])
        .await
        .unwrap();
    let query = MailQuery {
        account: Some("work".into()),
        folder: "INBOX".into(),
        sort: MailSort::Oldest,
        ..Default::default()
    };
    let id = MailSelectionId::default();
    store
        .capture_selection(id, 0, query, true, vec![])
        .await
        .unwrap();
    let frozen = store.freeze_selection(id, 0).await.unwrap();
    store.remove("work:INBOX:4".into()).await.unwrap();
    store
        .upsert(vec![
            message(1, "work", "INBOX"),
            message(3, "work", "INBOX"),
            message(5, "personal", "INBOX"),
        ])
        .await
        .unwrap();
    let passive = store
        .selection_snapshot(id, vec!["work:INBOX:1".into()])
        .await
        .unwrap();
    assert_eq!((passive.selected, passive.available), (3, 2));
    assert!(passive.visible.is_empty());
    let changed = store
        .change_selection(
            id,
            0,
            SelectionChange::Set {
                id: "work:INBOX:1".into(),
                selected: true,
                clear_others: false,
            },
            vec!["work:INBOX:1".into(), "work:INBOX:3".into()],
        )
        .await
        .unwrap();
    assert_eq!((changed.selected, changed.available), (4, 3));
    assert_eq!(changed.visible, ["work:INBOX:1".to_owned()].into());
    assert_eq!(
        selected_ids(&store, id, 1).await,
        ["work:INBOX:0", "work:INBOX:1", "work:INBOX:2"]
    );
    let review = store.selection_snapshot(frozen.id, vec![]).await.unwrap();
    assert_eq!((review.selected, review.available), (3, 2));
    assert!(
        store
            .change_selection(
                frozen.id,
                0,
                SelectionChange::Set {
                    id: "work:INBOX:1".into(),
                    selected: true,
                    clear_others: false
                },
                vec![]
            )
            .await
            .is_err()
    );
    // An out-of-scope click cannot clear valid choices, change the revision, or
    // leak its temporary rebase candidate when validation fails.
    assert!(
        store
            .change_selection(
                id,
                1,
                SelectionChange::Set {
                    id: "personal:INBOX:5".into(),
                    selected: true,
                    clear_others: true
                },
                vec![]
            )
            .await
            .is_err()
    );
    let kept = store.selection_snapshot(id, vec![]).await.unwrap();
    assert_eq!((kept.selected, kept.available, kept.revision), (4, 3, 1));
    let captures = store
        .run(|c| {
            Ok(
                c.query_row("SELECT COUNT(*) FROM temp.mail_selections", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert_eq!(captures, 2);
}

#[tokio::test]
async fn explicit_ranges_follow_current_order_including_intermediate_arrivals() {
    let store = Store::memory().unwrap();
    store
        .upsert(vec![
            message(0, "work", "INBOX"),
            message(2, "work", "INBOX"),
            message(4, "work", "INBOX"),
        ])
        .await
        .unwrap();
    let id = MailSelectionId::default();
    store
        .capture_selection(
            id,
            0,
            MailQuery {
                folder: "INBOX".into(),
                sort: MailSort::Oldest,
                ..Default::default()
            },
            false,
            vec![],
        )
        .await
        .unwrap();
    store
        .upsert(vec![
            message(1, "work", "INBOX"),
            message(3, "work", "INBOX"),
        ])
        .await
        .unwrap();
    let range = store
        .change_selection(
            id,
            0,
            SelectionChange::Range {
                anchor: "work:INBOX:0".into(),
                target: "work:INBOX:4".into(),
                additive: false,
            },
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(range.selected, 5);
    assert_eq!(
        selected_ids(&store, id, 1).await,
        (0..5)
            .map(|i| format!("work:INBOX:{i}"))
            .collect::<Vec<_>>()
    );
    let range = store
        .change_selection(
            id,
            1,
            SelectionChange::Range {
                anchor: "work:INBOX:0".into(),
                target: "work:INBOX:3".into(),
                additive: false,
            },
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(range.selected, 4);
    store.remove("work:INBOX:3".into()).await.unwrap();
    assert!(
        store
            .change_selection(
                id,
                2,
                SelectionChange::Range {
                    anchor: "work:INBOX:3".into(),
                    target: "work:INBOX:4".into(),
                    additive: false
                },
                vec![]
            )
            .await
            .is_err()
    );
    let kept = store.selection_snapshot(id, vec![]).await.unwrap();
    assert_eq!((kept.selected, kept.available, kept.revision), (4, 3, 2));
}
