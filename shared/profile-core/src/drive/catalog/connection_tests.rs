use super::*;

#[test]
fn catalog_and_each_nested_observation_use_the_owned_factory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.sqlite");
    let scope = Scope {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture".into(),
    };
    let (sent, received) = std::sync::mpsc::sync_channel(8);
    let factory = history::ConnectionFactory::new(move |path| {
        sent.send(path.to_owned()).unwrap();
        let c = Connection::open(path)?;
        c.execute_batch("PRAGMA temp_store=MEMORY;")?;
        Ok(c)
    });
    let mut catalog = Catalog::open_with(&path, scope.clone(), factory.clone()).unwrap();
    assert_eq!(received.recv().unwrap(), path.canonicalize().unwrap());
    let binding = Binding {
        namespace: scope.namespace.clone(),
        principal: scope.principal.clone(),
        profile: Uuid::new_v4(),
        generation: Uuid::new_v4(),
    };
    catalog.journal(binding.clone()).unwrap();
    let observation = received.recv().unwrap();
    assert!(observation.starts_with(&catalog.remote_root));
    catalog.journal(binding.clone()).unwrap();
    assert!(received.try_recv().is_err());
    let other = Binding {
        generation: Uuid::new_v4(),
        ..binding.clone()
    };
    catalog.journal(other).unwrap();
    assert_ne!(received.recv().unwrap(), observation);
    drop(catalog);
    let mut reopened = Catalog::open_with(&path, scope, factory).unwrap();
    assert_eq!(received.recv().unwrap(), path.canonicalize().unwrap());
    reopened.journal(binding).unwrap();
    assert_eq!(received.recv().unwrap(), observation);
}

#[tokio::test]
async fn failed_catalog_initializer_does_not_run_schema_or_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.sqlite");
    let scope = Scope {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture".into(),
    };
    let factory = history::ConnectionFactory::new(|_| Err(history::Error::Storage));
    assert!(matches!(
        Discovery::open_with(path.clone(), scope.clone(), factory).await,
        Err(Error::History(history::Error::Storage))
    ));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    Discovery::open(path, scope)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();
}

#[cfg(unix)]
#[test]
fn catalog_drop_releases_ownership_while_a_duplicate_description_survives() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.sqlite");
    let scope = Scope {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture".into(),
    };
    let catalog = Catalog::open(&path, scope.clone()).unwrap();
    let duplicate = catalog._lock.duplicate();
    assert!(matches!(
        Catalog::open(&path, scope.clone()),
        Err(Error::Owned)
    ));
    drop(catalog);
    let reopened = Catalog::open(&path, scope).unwrap();
    drop(duplicate);
    drop(reopened);
}
