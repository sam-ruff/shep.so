use super::*;

#[tokio::test]
async fn opening_a_newly_selected_profile_does_not_retarget_the_existing_session() {
    let directory = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let (original, session) = catalog.clone().open_active(false).await.unwrap();
    original.put("original", true).await.unwrap();
    let id = installed(&catalog, "Other profile").await;
    let revision = catalog.page(0).await.unwrap().revision;
    catalog.activate(Id::Imported(id), revision).await.unwrap();
    let snapshot = session.snapshot(0).await.unwrap();
    assert_eq!(snapshot.current.id, Id::Legacy);
    assert_eq!(snapshot.page.active, Id::Imported(id));
    original.put("saved-after-selection", true).await.unwrap();
    let (new_store, new_session) = catalog.clone().open_active(false).await.unwrap();
    assert_eq!(new_session.current, Id::Imported(id));
    assert!(
        !new_store
            .get::<bool>("saved-after-selection")
            .await
            .unwrap()
    );
    assert!(original.get::<bool>("saved-after-selection").await.unwrap());
    let revision = catalog.page(0).await.unwrap().revision;
    catalog.activate(Id::Legacy, revision).await.unwrap();
    let (reopened, _) = catalog.open_active(false).await.unwrap();
    assert!(reopened.get::<bool>("saved-after-selection").await.unwrap());
}

#[tokio::test]
async fn export_protects_the_catalog_other_profiles_and_their_journal_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let (_store, _) = catalog.clone().open_active(false).await.unwrap();
    let id = installed(&catalog, "Other profile").await;
    let imported = catalog.path(Id::Imported(id));
    for path in [
        catalog.root().join(CATALOG_FILE),
        catalog.root().join("profiles.sqlite-wal"),
        catalog.path(Id::Legacy),
        imported.clone(),
        imported.with_file_name("backup-uploads.sqlite"),
        imported.with_file_name("another.sqlite"),
    ] {
        assert!(catalog.check_export_destination(path).await.is_err());
    }
    assert!(
        catalog
            .check_export_destination(catalog.root().join("export.sqlite"))
            .await
            .is_ok()
    );
    #[cfg(unix)]
    {
        for (index, source) in [catalog.root().join(CATALOG_FILE), imported]
            .into_iter()
            .enumerate()
        {
            let target = catalog.root().join(format!("alias-{index}.sqlite"));
            std::fs::hard_link(&source, &target).unwrap();
            assert!(catalog.check_export_destination(target).await.is_err());
        }
    }
}

#[tokio::test]
async fn one_damaged_orphan_does_not_hide_successfully_recovered_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let _ = catalog.clone().open_active(false).await.unwrap();
    let id = installed(&catalog, "Keep accessible").await;
    let broken = catalog.path(Id::Imported(uuid::Uuid::new_v4()));
    std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
    std::fs::write(&broken, "Damaged copy, preserve it for recovery").unwrap();
    let report = catalog.recover_imports().await.unwrap();
    assert_eq!((report.found, report.warnings), (1, 1));
    assert!(report.message().unwrap().contains("kept"));
    let page = catalog.page(0).await.unwrap();
    assert_eq!(page.total, 2);
    assert!(
        page.rows
            .iter()
            .any(|p| p.id == Id::Imported(id) && p.ready)
    );
    assert_eq!(
        std::fs::read_to_string(broken).unwrap(),
        "Damaged copy, preserve it for recovery"
    );
}

async fn installed(catalog: &Catalog, name: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    catalog.reserve(id, name.into()).await.unwrap();
    let path = catalog.path(Id::Imported(id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let store = crate::store::Store::open(&path).unwrap();
    store
        .put(
            IMPORT_MARKER_KEY,
            ImportMarker {
                version: 1,
                local_profile: id,
                name: name.into(),
            },
        )
        .await
        .unwrap();
    catalog.finish(id).await.unwrap();
    id
}

#[tokio::test]
async fn legacy_cache_and_credentials_are_preserved_until_an_explicit_switch() {
    let directory = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let (revision, current) = catalog.active().await.unwrap();
    assert_eq!(current.id, Id::Legacy);
    assert_eq!(current.id.scope(), Scope::Legacy);
    assert_eq!(
        catalog.path(current.id),
        directory.path().join("shep.sqlite")
    );
    let id = installed(&catalog, "Imported work").await;
    assert_eq!(catalog.active().await.unwrap().1.id, Id::Legacy);
    assert!(catalog.activate(Id::Imported(id), revision).await.is_err());
    let page = catalog.page(0).await.unwrap();
    catalog
        .activate(Id::Imported(id), page.revision)
        .await
        .unwrap();
    let reopened = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    assert_eq!(reopened.active().await.unwrap().1.id, Id::Imported(id));
    assert_eq!(Id::Imported(id).scope(), Scope::Profile(id));
    assert_eq!(
        catalog.path(Id::Imported(id)),
        directory
            .path()
            .join("profiles")
            .join(id.to_string())
            .join("shep.sqlite")
    );
}

#[tokio::test]
async fn two_catalog_owners_reject_stale_activation_and_rename_without_losing_newer_intent() {
    let directory = tempfile::tempdir().unwrap();
    let first = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let second = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let id = installed(&first, "Work").await;
    let initial = first.page(0).await.unwrap();
    first
        .rename(Id::Imported(id), initial.revision, "Renamed work".into())
        .await
        .unwrap();
    assert!(
        second
            .rename(Id::Imported(id), initial.revision, "Old name".into())
            .await
            .is_err()
    );
    assert!(
        second
            .activate(Id::Imported(id), initial.revision)
            .await
            .is_err()
    );
    let (revision, _) = second.active().await.unwrap();
    second.activate(Id::Imported(id), revision).await.unwrap();
    assert_eq!(first.active().await.unwrap().1.name, "Renamed work");
    assert_eq!(first.page(0).await.unwrap().total, 2);
}

#[tokio::test]
async fn incomplete_imports_require_the_matching_prepared_file_and_registration_is_idempotent() {
    let directory = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let id = uuid::Uuid::new_v4();
    let first = catalog.reserve(id, "First name".into()).await.unwrap();
    assert_eq!(
        catalog
            .reserve(id, "Stale retry name".into())
            .await
            .unwrap(),
        first
    );
    assert!(!first.ready);
    let (revision, _) = catalog.active().await.unwrap();
    assert!(catalog.activate(Id::Imported(id), revision).await.is_err());
    assert!(catalog.finish(id).await.is_err());
    let path = catalog.path(Id::Imported(id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let store = crate::store::Store::open(path).unwrap();
    store
        .put(
            IMPORT_MARKER_KEY,
            ImportMarker {
                version: 1,
                local_profile: uuid::Uuid::new_v4(),
                name: "First name".into(),
            },
        )
        .await
        .unwrap();
    assert!(catalog.finish(id).await.is_err());
    store
        .put(
            IMPORT_MARKER_KEY,
            ImportMarker {
                version: 1,
                local_profile: id,
                name: "First name".into(),
            },
        )
        .await
        .unwrap();
    let ready = catalog.finish(id).await.unwrap();
    let revision = catalog.page(0).await.unwrap().revision;
    assert_eq!(catalog.finish(id).await.unwrap(), ready);
    assert_eq!(catalog.page(0).await.unwrap().revision, revision);
    assert_eq!(catalog.active().await.unwrap().1.id, Id::Legacy);
}

#[tokio::test]
async fn identity_names_and_metadata_pages_are_bounded_without_truncating_the_catalog() {
    for value in [
        "../outside",
        "/absolute",
        "",
        "00000000-0000-0000-0000-000000000000",
    ] {
        assert!(Id::parse(value).is_err());
    }
    let directory = tempfile::tempdir().unwrap();
    let catalog = Catalog::open(directory.path(), "shep.sqlite").unwrap();
    for index in 0..75 {
        catalog
            .reserve(uuid::Uuid::new_v4(), format!("Import {index:02}"))
            .await
            .unwrap();
    }
    assert!(
        catalog
            .reserve(uuid::Uuid::nil(), "Invalid".into())
            .await
            .is_err()
    );
    assert!(
        catalog
            .reserve(uuid::Uuid::new_v4(), "\n\t".into())
            .await
            .is_err()
    );
    let first = catalog.page(0).await.unwrap();
    let next = catalog.page(50).await.unwrap();
    assert_eq!(first.total, 76);
    assert_eq!(first.rows.len(), 50);
    assert_eq!(next.rows.len(), 26);
    assert!(
        first
            .rows
            .iter()
            .all(|a| next.rows.iter().all(|b| a.id != b.id))
    );
}

#[test]
fn occupied_catalog_filename_and_unsafe_legacy_paths_are_not_repurposed() {
    let directory = tempfile::tempdir().unwrap();
    let c = Connection::open(directory.path().join(CATALOG_FILE)).unwrap();
    c.execute_batch("CREATE TABLE original(value); INSERT INTO original VALUES('keep');")
        .unwrap();
    assert!(Catalog::open(directory.path(), "shep.sqlite").is_err());
    assert_eq!(
        c.query_row("SELECT value FROM original", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "keep"
    );
    assert!(Catalog::open(directory.path(), "../shep.sqlite").is_err());
}
