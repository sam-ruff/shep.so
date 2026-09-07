use shep::{
    folders::{Mailbox, NameEncoding},
    model::*,
    store::Store,
};
fn account() -> Account {
    serde_json::from_value(serde_json::json!({"id":"work","name":"Work","email":"work@example.test","protocol":"Imap","host":"localhost","port":993,"username":"work","smtp_host":"localhost","smtp_port":465})).unwrap()
}
fn folder(name: &str, delimiter: Option<char>, selectable: bool) -> Mailbox {
    Mailbox {
        name: name.into(),
        delimiter,
        selectable,
        encoding: NameEncoding::Utf8,
    }
}
#[tokio::test]
async fn catalog_and_expansion_survive_reopen_and_refresh_without_becoming_selectable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    store.save_account(account()).await.unwrap();
    store
        .save_folder_catalog(
            "work".into(),
            vec![
                folder("INBOX", Some('/'), true),
                folder("Teams/", Some('/'), false),
                folder("Teams/Remote/Plans", Some('/'), true),
            ],
        )
        .await
        .unwrap();
    // A retained cached message cannot turn an explicitly nonselectable parent
    // back into a selectable destination during workspace reconciliation.
    let mail = parse_mail(
        "work",
        "1",
        "Teams",
        b"Subject: Retained original\r\n\r\nBody".to_vec(),
        false,
        false,
    )
    .unwrap();
    let id = mail.summary.id.clone();
    store.upsert(vec![mail]).await.unwrap();
    let mut preferences = Preferences::default();
    preferences.expanded_folders.insert(
        "work".into(),
        ["Teams", "Teams/Remote"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    );
    store.save_preferences(preferences).await.unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let workspace = store.workspace().await.unwrap();
    assert_eq!(
        workspace.account_folders["work"],
        ["INBOX", "Teams/Remote/Plans"]
    );
    let tree = &workspace.folder_trees["work"];
    assert_eq!(
        tree.visible(workspace.preferences.expanded_folders.get("work"))
            .count(),
        4
    );
    assert!(!tree.node("Teams").unwrap().mailbox.selectable);
    assert!(store.raw_message(id).await.is_ok());
    store
        .save_folder_catalog(
            "work".into(),
            vec![
                folder("INBOX", Some('/'), true),
                folder("Teams", Some('/'), true),
                folder("Teams/Remote/Plans", Some('/'), true),
            ],
        )
        .await
        .unwrap();
    let workspace = store.workspace().await.unwrap();
    assert!(
        workspace.folder_trees["work"]
            .node("Teams")
            .unwrap()
            .mailbox
            .selectable
    );
    assert!(workspace.preferences.expanded_folders["work"].contains("Teams/Remote"));
}
#[tokio::test]
async fn legacy_cache_is_flat_until_the_server_supplies_its_delimiter() {
    let store = Store::memory().unwrap();
    store.save_account(account()).await.unwrap();
    store
        .save_folders(
            "work".into(),
            vec!["INBOX".into(), "Projects/Design".into(), "A.B".into()],
        )
        .await
        .unwrap();
    store
        .run(|connection| {
            connection.execute("DELETE FROM kv WHERE key='folder_catalogs'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let workspace = store.workspace().await.unwrap();
    assert_eq!(workspace.folder_trees["work"].roots.len(), 3);
    assert!(workspace.folder_trees["work"].node("Projects").is_none());
    store
        .save_folder_catalog(
            "work".into(),
            vec![
                folder("INBOX", Some('/'), true),
                folder("Projects/Design", Some('/'), true),
                folder("A.B", None, true),
            ],
        )
        .await
        .unwrap();
    let workspace = store.workspace().await.unwrap();
    assert!(
        !workspace.folder_trees["work"]
            .node("Projects")
            .unwrap()
            .mailbox
            .selectable
    );
    assert!(
        workspace.folder_trees["work"]
            .node("A.B")
            .unwrap()
            .parent
            .is_none()
    );
    assert!(workspace.preferences.expanded_folders.is_empty());
}
#[tokio::test]
async fn unicode_display_and_search_keep_the_exact_destination_identity() {
    let store = Store::memory().unwrap();
    store.save_account(account()).await.unwrap();
    let wire = "Projects/&ZeVnLIqe-";
    store
        .save_folder_catalog(
            "work".into(),
            vec![Mailbox {
                name: wire.into(),
                delimiter: Some('/'),
                selectable: true,
                encoding: NameEncoding::ImapUtf7,
            }],
        )
        .await
        .unwrap();
    let workspace = store.workspace().await.unwrap();
    assert_eq!(
        workspace.folder_label(Some("work"), wire),
        "Projects/日本語"
    );
    let found = shep::fuzzy::ranked_labels(
        "日本語",
        workspace.account_folders["work"].iter().map(|folder| {
            (
                folder.clone(),
                workspace.folder_label(Some("work"), folder).into_owned(),
            )
        }),
    );
    assert_eq!(found, [wire]);
    assert_eq!(workspace.folder_label(Some("other"), wire), wire);
}
