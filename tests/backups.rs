use async_trait::async_trait;
use shep::backup::{self, BackupCopy, BackupProvider};
use std::sync::Mutex;

struct Inventory {
    copies: Mutex<Vec<BackupCopy>>,
    deleted: Mutex<Vec<String>>,
}
impl Inventory {
    fn new(entries: &[(&str, &str)]) -> Self {
        Self {
            copies: Mutex::new(
                entries
                    .iter()
                    .map(|(id, name)| BackupCopy {
                        id: (*id).into(),
                        name: (*name).into(),
                        created_at: String::new(),
                    })
                    .collect(),
            ),
            deleted: Mutex::new(Vec::new()),
        }
    }
}
#[async_trait]
impl BackupProvider for Inventory {
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        Ok(self.copies.lock().unwrap().clone())
    }
    async fn upload(&self, _: &str, _: Vec<u8>) -> anyhow::Result<String> {
        unreachable!()
    }
    async fn download(&self, _: &str) -> anyhow::Result<Vec<u8>> {
        unreachable!()
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.deleted.lock().unwrap().push(id.into());
        self.copies.lock().unwrap().retain(|copy| copy.id != id);
        Ok(())
    }
}

#[tokio::test]
async fn retention_protects_the_committed_copy_when_the_clock_moves_backward() {
    let provider = Inventory::new(&[
        ("previous", "shep-20270906"),
        ("committed", "shep-20260906"),
    ]);
    assert_eq!(backup::retain(&provider, 1, "committed").await.unwrap(), 1);
    assert_eq!(*provider.deleted.lock().unwrap(), ["previous"]);
    assert_eq!(provider.list().await.unwrap()[0].id, "committed");
}

#[tokio::test]
async fn incomplete_or_duplicate_inventory_never_deletes_older_copies() {
    for entries in [
        vec![("previous", "older")],
        vec![
            ("committed", "one"),
            ("committed", "two"),
            ("previous", "older"),
        ],
    ] {
        let provider = Inventory::new(&entries);
        assert!(backup::retain(&provider, 1, "committed").await.is_err());
        assert!(provider.deleted.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn local_upload_cannot_replace_existing_copies_or_leave_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    let provider = backup::LocalBackup {
        directory: directory.path().into(),
    };
    let name = format!("shep-20260906T120000Z-{}.shepbackup", uuid::Uuid::nil());
    provider.upload(&name, b"original".to_vec()).await.unwrap();
    assert!(
        provider
            .upload(&name, b"replacement".to_vec())
            .await
            .is_err()
    );
    assert_eq!(provider.download(&name).await.unwrap(), b"original");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    let unrelated = "shep-personal-notes.shepbackup";
    tokio::fs::write(directory.path().join(unrelated), b"keep me")
        .await
        .unwrap();
    assert_eq!(provider.list().await.unwrap().len(), 1);
    assert!(provider.delete(unrelated).await.is_err());
    assert!(
        provider
            .upload("shep-20260900T120000Z-invalid.shepbackup", Vec::new())
            .await
            .is_err()
    );
    provider.delete(&name).await.unwrap();
    provider.delete(&name).await.unwrap();
    assert!(directory.path().join(unrelated).exists());
}
