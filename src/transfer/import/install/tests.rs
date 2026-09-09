use super::*;

async fn setup() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    Store,
    Catalog,
    Prepared,
) {
    let original = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let path = original.path().join("source.sqlite");
    let source = super::super::tests::workspace(&path).await;
    let store = Store::open(local.path().join("shep.sqlite")).unwrap();
    let catalog = Catalog::open(local.path(), "shep.sqlite").unwrap();
    let prepared = stage(store, path)
        .await
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    (original, local, source, catalog, prepared)
}

#[tokio::test]
async fn install_consumes_the_reviewed_copy_keeps_the_original_and_registers_without_switching() {
    let (_original, _local, source, catalog, prepared) = setup().await;
    source.put("later-source-change", true).await.unwrap();
    let old_scope = catalog.active().await.unwrap().1.id.scope();
    let mut install = prepared
        .install(
            catalog.clone(),
            "Imported mail".into(),
            Preferences::default(),
        )
        .unwrap();
    let saved = install.finish().await.unwrap().unwrap();
    assert!(saved.registered);
    assert!(saved.warning.is_none(), "{:?}", saved.warning);
    assert_ne!(saved.id.scope(), old_scope);
    assert_eq!(catalog.active().await.unwrap().1.id, Id::Legacy);
    let imported = Store::open(&saved.path).unwrap();
    assert!(!imported.get::<bool>("later-source-change").await.unwrap());
    assert!(source.get::<bool>("later-source-change").await.unwrap());
    assert_eq!(imported.query(Default::default()).await.unwrap().total, 1);
    assert_eq!(imported.get::<Vec<Draft>>("drafts").await.unwrap().len(), 1);
    let page = catalog.page(0).await.unwrap();
    catalog.activate(saved.id, page.revision).await.unwrap();
    let reopened = Catalog::open(catalog.root(), "shep.sqlite").unwrap();
    assert_eq!(reopened.active().await.unwrap().1.id, saved.id);
}

#[tokio::test]
async fn cancel_before_publication_cleans_up_but_cancel_after_publication_keeps_the_receipt() {
    for phase in [InstallPhase::Publishing, InstallPhase::Registering] {
        let (_original, _local, _source, catalog, prepared) = setup().await;
        let id = prepared.id;
        let (entered, waiting) = oneshot::channel();
        let (release, held) = oneshot::channel();
        let mut gate = Some((entered, held));
        let mut install = prepared
            .install_observed(
                catalog.clone(),
                "Cancellation".into(),
                Preferences::default(),
                move |current| {
                    if current == phase
                        && let Some((entered, held)) = gate.take()
                    {
                        entered.send(()).unwrap();
                        held.blocking_recv().unwrap();
                    }
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap();
        assert!(futures::FutureExt::now_or_never(install.finish()).is_none());
        install.cancel();
        release.send(()).unwrap();
        let result = install.finish().await.unwrap();
        if phase == InstallPhase::Publishing {
            assert!(result.is_none());
            assert!(!catalog.path(Id::Imported(id)).parent().unwrap().exists());
            assert_eq!(catalog.page(0).await.unwrap().total, 1);
        } else {
            let saved = result.unwrap();
            assert!(saved.path.is_file());
            assert!(saved.registered);
            assert_eq!(catalog.page(0).await.unwrap().total, 2);
        }
    }
}

#[tokio::test]
async fn a_completed_copy_is_recovered_after_losing_its_registration_without_reimporting() {
    let (_original, _local, _source, catalog, prepared) = setup().await;
    let id = prepared.id;
    let (_keep_alive, cancel) = watch::channel(false);
    let saved = publish(
        prepared,
        catalog.path(Id::Imported(id)),
        "Recovered import",
        &Preferences::default(),
        &cancel,
        |_| {},
    )
    .unwrap()
    .unwrap();
    assert!(!saved.registered);
    assert_eq!(catalog.page(0).await.unwrap().total, 1);
    let reopened = Catalog::open(catalog.root(), "shep.sqlite").unwrap();
    assert_eq!(reopened.recover_imports().await.unwrap().found, 1);
    let page = reopened.page(0).await.unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.active, Id::Legacy);
    reopened
        .rename(saved.id, page.revision, "New name".into())
        .await
        .unwrap();
    let revision = reopened.page(0).await.unwrap().revision;
    reopened.recover_imports().await.unwrap();
    let page = reopened.page(0).await.unwrap();
    assert_eq!(page.revision, revision);
    assert!(
        page.rows
            .iter()
            .any(|p| p.id == saved.id && p.name == "New name" && p.ready)
    );
}

#[tokio::test]
async fn destination_collisions_and_missing_registered_files_never_replace_another_profile() {
    let (_original, _local, _source, catalog, prepared) = setup().await;
    let path = catalog.path(Id::Imported(prepared.id));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "Keep existing file").unwrap();
    let mut install = prepared
        .install(catalog.clone(), "Collision".into(), Preferences::default())
        .unwrap();
    assert!(install.finish().await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"Keep existing file");
    assert_eq!(catalog.page(0).await.unwrap().total, 1);
    assert_eq!(catalog.recover_imports().await.unwrap().warnings, 1);

    let (_original, _local, _source, catalog, prepared) = setup().await;
    let saved = prepared
        .install(
            catalog.clone(),
            "Missing file".into(),
            Preferences::default(),
        )
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    std::fs::remove_file(&saved.path).unwrap();
    let page = catalog.page(0).await.unwrap();
    assert!(catalog.activate(saved.id, page.revision).await.is_err());
    assert_eq!(catalog.active().await.unwrap().1.id, Id::Legacy);
    assert!(!saved.path.exists());
}
