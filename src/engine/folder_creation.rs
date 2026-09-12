use super::*;
use crate::{folder_actions::Connection, folders::Mailbox};

#[derive(Debug, Clone)]
pub struct Request {
    pub serial: u64,
    pub account: String,
    pub connection: String,
    pub parent: Option<String>,
    pub name: String,
}

type Confirmed = (Mailbox, Vec<Mailbox>);

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
trait CreationProvider: Send + Sync {
    async fn ensure(&self, target: Mailbox) -> anyhow::Result<Confirmed>;
    async fn inspect(&self, target: Mailbox) -> anyhow::Result<Option<Confirmed>>;
}

async fn confirm(provider: &impl CreationProvider, target: &Mailbox) -> anyhow::Result<Confirmed> {
    match provider.ensure(target.clone()).await {
        Ok(confirmed) => Ok(confirmed),
        Err(original) => match provider.inspect(target.clone()).await? {
            Some(confirmed) => Ok(confirmed),
            None => Err(original),
        },
    }
}

struct ImapCreation<'a> {
    account: &'a Account,
    password: &'a secrecy::SecretString,
}

#[async_trait::async_trait]
impl CreationProvider for ImapCreation<'_> {
    async fn ensure(&self, target: Mailbox) -> anyhow::Result<Confirmed> {
        let mut connection =
            providers::mail::folders::ImapFolders::open(self.account, self.password).await?;
        let created = connection.ensure_planned_folder(&target).await?;
        Ok((created, connection.catalog().await?))
    }

    async fn inspect(&self, target: Mailbox) -> anyhow::Result<Option<Confirmed>> {
        let mut connection =
            providers::mail::folders::ImapFolders::open(self.account, self.password)
                .await
                .context(
                    "The folder could not be confirmed. Try again to check the saved destination.",
                )?;
        match connection.find_planned_folder(&target).await? {
            Some(created) => Ok(Some((created, connection.catalog().await?))),
            None => Ok(None),
        }
    }
}

impl Engine {
    pub(super) async fn create_folder(
        &self,
        request: Request,
        mut output: Output,
    ) -> anyhow::Result<()> {
        let serial = request.serial;
        let result = async {
            let created = self.perform_create_folder(request.clone()).await?;
            self.workspace(&mut output).await
                .context("The folder was created, but its list could not be refreshed. Try again to check it.")?;
            self.store.finish_folder_creation(request.account, request.connection,
                serde_json::to_string(&(&request.parent, &request.name))?).await?;
            Ok::<_, anyhow::Error>(created)
        }.await;
        if result.is_err() {
            let _ = self.workspace(&mut output).await;
        }
        output
            .send(Event::FolderCreated(
                serial,
                result.map_err(|error| format!("{error:#}")),
            ))
            .await?;
        Ok(())
    }

    async fn perform_create_folder(&self, request: Request) -> anyhow::Result<Mailbox> {
        let _account_lock = self.account_access(&request.account).await;
        let account = self.account(&request.account).await?;
        anyhow::ensure!(
            crate::mail_actions::connection_key(&account) == request.connection,
            "This account's connection changed. Choose the account again before creating a folder."
        );
        self.store.ensure_folder_idle(account.id.clone()).await?;
        if account.protocol == Protocol::Pop3 {
            return self
                .store
                .create_local_folder(account.id, request.connection, request.parent, request.name)
                .await;
        }
        let request_key = serde_json::to_string(&(&request.parent, &request.name))?;
        let saved = self
            .store
            .folder_creation_target(
                account.id.clone(),
                request.connection.clone(),
                request_key.clone(),
                None,
            )
            .await?;
        let (created, catalog) = if self.demo {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.preview_create_folder(&request, saved, &request_key)
                    .await?
            }
            #[cfg(not(any(test, feature = "test-support")))]
            {
                anyhow::bail!("Folder creation preview is unavailable in this build.");
            }
        } else {
            let password = self.credentials.read(&account.id).await?;
            let target = match saved {
                Some(target) => target,
                None => {
                    let mut connection =
                        providers::mail::folders::ImapFolders::open(&account, &password).await?;
                    connection
                        .plan_folder(request.parent.as_deref(), &request.name)
                        .await?
                }
            };
            let target = self
                .store
                .folder_creation_target(
                    account.id.clone(),
                    request.connection.clone(),
                    request_key.clone(),
                    Some(target),
                )
                .await?
                .context("The folder request was not saved.")?;
            confirm(
                &ImapCreation {
                    account: &account,
                    password: &password,
                },
                &target,
            )
            .await?
        };
        self.store
            .save_created_folder_catalog(
                account.id,
                request.connection,
                catalog,
                created.clone(),
                request_key,
            )
            .await?;
        Ok(created)
    }

    #[cfg(any(test, feature = "test-support"))]
    async fn preview_create_folder(
        &self,
        request: &Request,
        saved_target: Option<Mailbox>,
        request_key: &str,
    ) -> anyhow::Result<(Mailbox, Vec<Mailbox>)> {
        let key = format!("fixture:folder-create:{}", request.account);
        let saved = self.store.get::<Option<Vec<Mailbox>>>(&key).await?;
        let catalog = match saved {
            Some(catalog) => catalog,
            None => {
                self.store
                    .current_folder_catalog(request.account.clone())
                    .await?
            }
        };
        let root = Mailbox {
            delimiter: catalog.first().and_then(|mailbox| mailbox.delimiter),
            encoding: catalog
                .first()
                .map(|mailbox| mailbox.encoding)
                .unwrap_or_default(),
            ..Mailbox::flat(String::new())
        };
        let parent = request
            .parent
            .as_ref()
            .map(|path| {
                catalog
                    .iter()
                    .find(|mailbox| &mailbox.name == path)
                    .context("The parent folder is no longer available.")
            })
            .transpose()?;
        let target = match saved_target {
            Some(target) => target,
            None => crate::folder_actions::creation::plan(&root, parent, &request.name)?,
        };
        let created = self
            .store
            .folder_creation_target(
                request.account.clone(),
                request.connection.clone(),
                request_key.into(),
                Some(target),
            )
            .await?
            .context("The folder request was not saved.")?;
        let mode = std::env::args()
            .find_map(|arg| arg.strip_prefix("--folder-actions=").map(str::to_owned))
            .unwrap_or_default();
        confirm(
            &PreviewCreation {
                store: &self.store,
                key,
                catalog,
                created: created.clone(),
                mode,
            },
            &created,
        )
        .await
    }
}

#[cfg(any(test, feature = "test-support"))]
struct PreviewCreation<'a> {
    store: &'a Store,
    key: String,
    catalog: Vec<Mailbox>,
    created: Mailbox,
    mode: String,
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait::async_trait]
impl CreationProvider for PreviewCreation<'_> {
    async fn ensure(&self, target: Mailbox) -> anyhow::Result<Confirmed> {
        if let Some(existing) = self.inspect(target).await? {
            return Ok(existing);
        }
        if !self.mode.is_empty() {
            tokio::time::sleep(Duration::from_millis(1600)).await;
        }
        let attempted = format!("{}:attempted", self.key);
        let first = !self.store.get::<bool>(&attempted).await?;
        self.store.put(&attempted, true).await?;
        anyhow::ensure!(
            !(first && self.mode == "fail"),
            "The fictional server refused this folder. Try again."
        );
        let mut catalog = self.catalog.clone();
        catalog.push(self.created.clone());
        self.store.put(&self.key, Some(catalog.clone())).await?;
        anyhow::ensure!(
            !(first && self.mode == "uncertain"),
            "The connection closed before confirming the folder."
        );
        Ok((self.created.clone(), catalog))
    }

    async fn inspect(&self, target: Mailbox) -> anyhow::Result<Option<Confirmed>> {
        let catalog = self
            .store
            .get::<Option<Vec<Mailbox>>>(&self.key)
            .await?
            .unwrap_or_else(|| self.catalog.clone());
        let Some(created) = catalog
            .iter()
            .find(|mailbox| mailbox.name == target.name)
            .cloned()
        else {
            return Ok(None);
        };
        anyhow::ensure!(
            created.selectable && !created.non_existent,
            "This folder is not available."
        );
        Ok(Some((created, catalog)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::eq;

    fn confirmed() -> Confirmed {
        let mailbox = Mailbox {
            delimiter: Some('.'),
            encoding: crate::folders::NameEncoding::ImapUtf7,
            ..Mailbox::flat("INBOX.&ZeVnLIqe-".into())
        };
        (mailbox.clone(), vec![mailbox])
    }

    #[tokio::test]
    async fn success_needs_no_reconnect() {
        let mut provider = MockCreationProvider::new();
        provider
            .expect_ensure()
            .with(eq(confirmed().0))
            .times(1)
            .returning(|_| Ok(confirmed()));
        provider.expect_inspect().never();
        assert_eq!(
            confirm(&provider, &confirmed().0).await.expect("confirmed"),
            confirmed()
        );
    }

    #[tokio::test]
    async fn lost_reply_uses_read_only_exact_target_without_a_second_create() {
        let mut provider = MockCreationProvider::new();
        let mut sequence = mockall::Sequence::new();
        provider
            .expect_ensure()
            .with(eq(confirmed().0))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| anyhow::bail!("reply lost"));
        provider
            .expect_inspect()
            .with(eq(confirmed().0))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Ok(Some(confirmed())));
        assert_eq!(
            confirm(&provider, &confirmed().0)
                .await
                .expect("read-back confirmed"),
            confirmed()
        );
    }

    #[tokio::test]
    async fn absent_or_unreadable_target_cannot_claim_success_or_repeat_create() {
        for unreadable in [false, true] {
            let mut provider = MockCreationProvider::new();
            provider
                .expect_ensure()
                .times(1)
                .returning(|_| anyhow::bail!("create rejected"));
            provider
                .expect_inspect()
                .with(eq(confirmed().0))
                .times(1)
                .returning(move |_| {
                    if unreadable {
                        anyhow::bail!("inspection unavailable")
                    } else {
                        Ok(None)
                    }
                });
            let error = confirm(&provider, &confirmed().0)
                .await
                .expect_err("unconfirmed");
            assert_eq!(
                error.to_string(),
                if unreadable {
                    "inspection unavailable"
                } else {
                    "create rejected"
                }
            );
        }
    }
}
