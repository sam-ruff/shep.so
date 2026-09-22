pub mod condstore;
#[cfg(all(test, feature = "condstore"))]
mod condstore_tests;
pub mod folders;
pub mod gateway;
#[cfg(test)]
mod lane_tests;
mod lanes;
#[cfg(test)]
mod notification_tests;
pub mod push;
#[cfg(test)]
mod push_tests;
pub mod receipts;
pub mod recovery;
pub mod sent;
#[cfg(feature = "staged-receive")]
pub mod staging;
#[cfg(all(test, feature = "staged-receive"))]
mod staging_tests;
#[cfg(any(test, feature = "staged-receive"))]
mod streaming;
mod sync_queries;
use super::MailProvider;
use crate::model::*;
use anyhow::Context;
use async_trait::async_trait;
use futures::TryStreamExt;
use secrecy::{ExposeSecret, SecretString};
use std::{collections::HashSet, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::mpsc::Sender,
};

pub struct Imap;
pub struct Pop3;
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReceiveMode {
    Legacy,
    #[cfg(feature = "staged-receive")]
    Staged,
    Protected,
}
pub fn provider(protocol: Protocol) -> Box<dyn MailProvider> {
    match protocol {
        Protocol::Imap => Box::new(Imap),
        Protocol::Pop3 => Box::new(Pop3),
    }
}
pub type Tls = tokio_native_tls::TlsStream<TcpStream>;

async fn tls(host: &str, port: u16, route: &gateway::Route) -> anyhow::Result<Tls> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let tcp = route
            .connect(host, port)
            .await
            .context("Could not reach the mail server")?;
        let connector = route.connector()?;
        connector
            .connect(host, tcp)
            .await
            .context("The server's TLS certificate could not be verified")
    })
    .await
    .context("The mail server took too long to connect")?
}

async fn upgrade(host: &str, tcp: TcpStream, route: &gateway::Route) -> anyhow::Result<Tls> {
    let connector = route.connector()?;
    connector
        .connect(host, tcp)
        .await
        .context("TLS certificate validation failed")
}
struct PlainAuth<'a> {
    username: &'a str,
    password: &'a str,
}
impl async_imap::Authenticator for PlainAuth<'_> {
    type Response = Vec<u8>;
    fn process(&mut self, _: &[u8]) -> Self::Response {
        format!("\0{}\0{}", self.username, self.password).into_bytes()
    }
}
/// Authenticated direct IMAP session for client-owned adapters such as durable
/// move connections. Routed connections stay internal to the gateway.
pub async fn imap(
    account: &Account,
    password: &SecretString,
) -> anyhow::Result<async_imap::Session<Tls>> {
    imap_routed(account, password, &gateway::Route::default()).await
}
async fn imap_routed(
    account: &Account,
    password: &SecretString,
    route: &gateway::Route,
) -> anyhow::Result<async_imap::Session<Tls>> {
    let client = if account.incoming_security == ConnectionSecurity::StartTls {
        let tcp = tokio::time::timeout(
            Duration::from_secs(20),
            route.connect(&account.host, account.port),
        )
        .await??;
        let mut client = async_imap::Client::new(tcp);
        client
            .read_response()
            .await?
            .context("Missing IMAP greeting")?;
        client
            .run_command_and_check_ok("STARTTLS", None)
            .await
            .context("The server refused STARTTLS")?;
        async_imap::Client::new(upgrade(&account.host, client.into_inner(), route).await?)
    } else {
        let mut client = async_imap::Client::new(tls(&account.host, account.port, route).await?);
        client
            .read_response()
            .await?
            .context("The IMAP server closed the connection")?;
        client
    };
    match account.incoming_auth {
        IncomingAuth::Password => {
            client
                .login(&account.username, password.expose_secret())
                .await
        }
        IncomingAuth::Plain => {
            client
                .authenticate(
                    "PLAIN",
                    PlainAuth {
                        username: &account.username,
                        password: password.expose_secret(),
                    },
                )
                .await
        }
    }
    .map_err(|(e, _)| anyhow::anyhow!("IMAP authentication failed: {e}"))
}
async fn pop(account: &Account, password: &SecretString) -> anyhow::Result<PopConnection<Tls>> {
    pop_routed(account, password, &gateway::Route::default()).await
}
async fn pop_routed(
    account: &Account,
    password: &SecretString,
    route: &gateway::Route,
) -> anyhow::Result<PopConnection<Tls>> {
    let mut connection = if account.incoming_security == ConnectionSecurity::StartTls {
        let tcp = tokio::time::timeout(
            Duration::from_secs(20),
            route.connect(&account.host, account.port),
        )
        .await??;
        let mut plain = PopConnection(BufReader::new(tcp));
        anyhow::ensure!(
            plain.line().await?.starts_with("+OK"),
            "Invalid POP3 greeting"
        );
        plain
            .command("STLS")
            .await
            .context("The POP3 server refused STARTTLS")?;
        anyhow::ensure!(
            plain.0.buffer().is_empty(),
            "Unexpected data before TLS negotiation"
        );
        PopConnection(BufReader::new(
            upgrade(&account.host, plain.0.into_inner(), route).await?,
        ))
    } else {
        let mut connection = PopConnection(BufReader::new(
            tls(&account.host, account.port, route).await?,
        ));
        anyhow::ensure!(
            connection.line().await?.starts_with("+OK"),
            "Invalid POP3 greeting"
        );
        connection
    };
    match account.incoming_auth {
        IncomingAuth::Password => {
            connection
                .command(&format!("USER {}", account.username))
                .await?;
            connection
                .command(&format!("PASS {}", password.expose_secret()))
                .await?;
        }
        IncomingAuth::Plain => {
            use base64::Engine;
            let value = base64::engine::general_purpose::STANDARD.encode(format!(
                "\0{}\0{}",
                account.username,
                password.expose_secret()
            ));
            connection.command(&format!("AUTH PLAIN {value}")).await?;
        }
    }
    Ok(connection)
}

/// Public for client test harnesses that drive an in-memory IMAP session.
pub async fn sync_imap_session<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    session: async_imap::Session<T>,
    account: &Account,
    known: &HashSet<String>,
    output: Sender<MailSyncItem>,
    only_folder: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    sync_imap_session_mode(
        session,
        account,
        known,
        output,
        only_folder,
        ReceiveMode::Legacy,
        None,
    )
    .await
}

/// `resume` enables CONDSTORE flag refresh from the caller's saved folder
/// states; `None` keeps the full flag listing for callers that cannot
/// persist those states.
async fn sync_imap_session_mode<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    mut session: async_imap::Session<T>,
    account: &Account,
    known: &HashSet<String>,
    output: Sender<MailSyncItem>,
    only_folder: Option<&str>,
    mode: ReceiveMode,
    resume: Option<&condstore::Resume>,
) -> anyhow::Result<Vec<String>> {
    let mut slow: Vec<lanes::Deferred> = Vec::new();
    let mut staged: Vec<lanes::Deferred> = Vec::new();
    let capabilities = session.capabilities().await?;
    let pattern = if capabilities.has_str("SPECIAL-USE") {
        "\"*\" RETURN (SPECIAL-USE)"
    } else {
        "*"
    };
    let names: Vec<_> = session
        .list(None, Some(pattern))
        .await?
        .try_collect()
        .await?;
    output
        .send(MailSyncItem::SentFolder(
            account.id.clone(),
            sent::choose_folder(&names, &account.sent_folder).ok(),
        ))
        .await?;
    let mut catalog: Vec<_> = names
        .iter()
        .map(|name| {
            folders::mailbox(
                name.name(),
                name.delimiter(),
                name.attributes(),
                folders::encoding(&capabilities),
            )
        })
        .collect();
    catalog.sort_by_key(|folder| !folder.name.eq_ignore_ascii_case("INBOX"));
    let folders: Vec<String> = catalog
        .iter()
        .filter(|folder| folder.selectable)
        .map(|folder| folder.name.clone())
        .collect();
    output
        .send(MailSyncItem::Folders(account.id.clone(), catalog))
        .await?;
    tracing::info!(folders = folders.len(), "IMAP folder listing complete");
    for (index, folder) in folders.iter().enumerate() {
        if only_folder.is_some_and(|wanted| !folder.eq_ignore_ascii_case(wanted)) {
            continue;
        }
        tracing::info!(folder_index = index, "Selecting IMAP folder");
        let saved = resume.and_then(|resume| resume.get(folder));
        let modseqs = resume.is_some() && capabilities.has_str("CONDSTORE");
        let mailbox = if modseqs {
            session.select_condstore(folder).await?
        } else {
            session.select(folder).await?
        };
        let validity = mailbox
            .uid_validity
            .context("The server did not provide UIDVALIDITY")?;
        let highest = condstore::usable(mailbox.highest_modseq).filter(|_| modseqs);
        let plan = condstore::plan(saved, validity, highest);
        if folder.eq_ignore_ascii_case("INBOX") {
            output
                .send(MailSyncItem::InboxSyncStarted {
                    account: account.id.clone(),
                    epoch: format!("imap:{validity}"),
                })
                .await?;
        }
        // Fetch bounded metadata batches before requesting message bodies.
        let mut uids: Vec<_> = sync_queries::search(&mut session)
            .await?
            .into_iter()
            .collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        tracing::info!(
            folder_index = index,
            messages = uids.len(),
            "IMAP UID search complete"
        );
        let live_ids: HashSet<String> = uids
            .iter()
            .map(|uid| format!("{}:{folder}:{validity}.{uid}", account.id))
            .collect();
        let changed = match plan {
            condstore::Plan::Full => None,
            condstore::Plan::Unchanged => Some(Vec::new()),
            condstore::Plan::Changes { since } => {
                match condstore::changed_flags(&mut session, since).await {
                    Ok(changed) => Some(changed),
                    Err(error) => {
                        tracing::warn!(
                            "CONDSTORE flag refresh failed, listing every flag: {error:#}"
                        );
                        None
                    }
                }
            }
        };
        let listed: Vec<u32> = match &changed {
            // Cached messages are covered by the changed flags; only new
            // messages need their metadata.
            Some(changed) => {
                let flags: Vec<_> = changed
                    .iter()
                    .filter_map(|fetch| {
                        let id = format!("{}:{folder}:{validity}.{}", account.id, fetch.uid?);
                        (known.contains(&id) && live_ids.contains(&id)).then_some((
                            id,
                            fetch.unread,
                            fetch.starred,
                        ))
                    })
                    .collect();
                for chunk in flags.chunks(50) {
                    output.send(MailSyncItem::Flags(chunk.to_vec())).await?;
                }
                uids.iter()
                    .copied()
                    .filter(|uid| {
                        !known.contains(&format!("{}:{folder}:{validity}.{uid}", account.id))
                    })
                    .collect()
            }
            None => uids,
        };
        for chunk in listed.chunks(50) {
            let set = chunk
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let metadata =
                sync_queries::fetch(&mut session, &set, "(UID FLAGS RFC822.SIZE)").await?;
            let mut pending = Vec::new();
            let mut flags = Vec::new();
            for fetch in &metadata {
                let Some(uid) = fetch.uid else {
                    continue;
                };
                let id = format!("{}:{folder}:{validity}.{uid}", account.id);
                if known.contains(&id) {
                    flags.push((id, fetch.unread, fetch.starred));
                    continue;
                }
                let deferred = |size: u32| lanes::Deferred {
                    folder_index: index,
                    folder: folder.clone(),
                    validity,
                    uid,
                    size,
                    unread: fetch.unread,
                    starred: fetch.starred,
                };
                match lanes::lane(fetch.size) {
                    lanes::Lane::Inline => pending.push(lanes::Pending {
                        uid,
                        size: fetch.size.unwrap_or(0) as usize,
                    }),
                    lanes::Lane::Slow => slow.push(deferred(fetch.size.unwrap_or(0))),
                    lanes::Lane::Staged if mode == ReceiveMode::Legacy => {
                        output.send(MailSyncItem::SkippedLarge).await?;
                    }
                    lanes::Lane::Staged => {
                        staged.push(deferred(
                            fetch.size.context("The message size is missing.")?,
                        ));
                    }
                }
            }
            if !flags.is_empty() {
                output.send(MailSyncItem::Flags(flags)).await?;
            }
            lanes::order_inline(&mut pending);
            tracing::info!(count = pending.len(), "IMAP metadata received");
            for range in lanes::batches(&pending) {
                let batch: Vec<u32> = pending[range].iter().map(|item| item.uid).collect();
                let set = batch
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let bodies =
                    sync_queries::fetch(&mut session, &set, "(UID FLAGS BODY.PEEK[])").await?;
                deliver_bodies(account, folder, validity, &batch, &bodies, &output).await?;
            }
        }
        // The complete listing: the store only removes cached rows absent from
        // it, and a deferred body is not cached yet, so listing it is harmless.
        output
            .send(MailSyncItem::Reconcile {
                account: account.id.clone(),
                folder: folder.clone(),
                live_ids,
            })
            .await?;
        // Saved only after the folder's flags were sent, so an interrupted
        // check never advances past changes it did not deliver.
        match highest {
            Some(modseq) => {
                let state = condstore::FolderState { validity, modseq };
                if saved != Some(&state) {
                    condstore::publish(&output, &account.id, folder, Some(state)).await?;
                }
            }
            None if saved.is_some() => {
                condstore::publish(&output, &account.id, folder, None).await?;
            }
            _ => {}
        }
        if folder.eq_ignore_ascii_case("INBOX")
            && lanes::inbox_finish(&slow, &staged) == lanes::InboxFinish::Inline
        {
            output
                .send(MailSyncItem::InboxSyncFinished {
                    account: account.id.clone(),
                    epoch: format!("imap:{validity}"),
                })
                .await?;
        }
    }
    // Slow lane: one body per request, after every folder's small mail. A
    // failure here leaves the earlier messages committed; the remaining
    // deferred messages are still unknown and are fetched again next check.
    lanes::order_slow(&mut slow);
    let finish = lanes::inbox_finish(&slow, &staged);
    let last_inbox_slow = slow.iter().rposition(lanes::Deferred::inbox);
    let mut selected: Option<&str> = None;
    for (position, item) in slow.iter().enumerate() {
        if selected != Some(item.folder.as_str()) {
            anyhow::ensure!(
                session.select(&item.folder).await?.uid_validity == Some(item.validity),
                "The mailbox changed during download. Refresh and retry."
            );
            selected = Some(item.folder.as_str());
        }
        tracing::info!(uid = item.uid, size = item.size, "IMAP slow-lane body");
        let bodies = sync_queries::fetch(
            &mut session,
            &item.uid.to_string(),
            "(UID FLAGS BODY.PEEK[])",
        )
        .await?;
        deliver_bodies(
            account,
            &item.folder,
            item.validity,
            &[item.uid],
            &bodies,
            &output,
        )
        .await?;
        if finish == lanes::InboxFinish::SlowLane && Some(position) == last_inbox_slow {
            output
                .send(MailSyncItem::InboxSyncFinished {
                    account: account.id.clone(),
                    epoch: format!("imap:{}", item.validity),
                })
                .await?;
        }
    }
    anyhow::ensure!(
        mode != ReceiveMode::Protected || staged.is_empty(),
        "Large-message downloads need encrypted staging for this encrypted profile. Smaller messages were downloaded; the originals remain on the server."
    );
    #[cfg(feature = "staged-receive")]
    for item in &staged {
        anyhow::ensure!(
            session.select(&item.folder).await?.uid_validity == Some(item.validity),
            "The mailbox changed during download. Refresh and retry."
        );
        let source = tempfile::NamedTempFile::new()?;
        let mut writer = tokio::fs::File::from_std(source.reopen()?);
        streaming::download_reporting(
            &mut session,
            item.uid,
            item.size,
            &mut writer,
            Some(&output),
        )
        .await?;
        drop(writer);
        let (account, folder, remote, unread, starred) = (
            account.id.clone(),
            item.folder.clone(),
            format!("{}.{}", item.validity, item.uid),
            item.unread,
            item.starred,
        );
        let message = tokio::task::spawn_blocking(move || {
            staging::prepare(source, &account, &remote, &folder, unread, starred)
        })
        .await??;
        output
            .send(MailSyncItem::StagedMessage(message))
            .await
            .context("Sync was cancelled")?;
    }
    if finish == lanes::InboxFinish::Staged
        && let Some(item) = staged.iter().find(|item| item.inbox())
    {
        output
            .send(MailSyncItem::InboxSyncFinished {
                account: account.id.clone(),
                epoch: format!("imap:{}", item.validity),
            })
            .await?;
    }
    session.logout().await?;
    Ok(folders)
}

/// Parse and publish the requested bodies from one FETCH reply. A missing body
/// means the message left the folder after listing; the next check notices.
async fn deliver_bodies(
    account: &Account,
    folder: &str,
    validity: u32,
    requested: &[u32],
    bodies: &[sync_queries::Fetch],
    output: &Sender<MailSyncItem>,
) -> anyhow::Result<()> {
    for fetch in bodies {
        let (Some(uid), Some(raw)) = (fetch.uid, fetch.body.as_deref()) else {
            continue;
        };
        if !requested.contains(&uid) {
            continue;
        }
        anyhow::ensure!(
            raw.len() <= MAX_MESSAGE_BYTES,
            "The server returned a message exceeding 25 MiB."
        );
        let (unread, starred) = (fetch.unread, fetch.starred);
        let (id, folder, remote, raw) = (
            account.id.clone(),
            folder.to_owned(),
            format!("{validity}.{uid}"),
            raw.to_vec(),
        );
        let parsed = tokio::task::spawn_blocking(move || {
            parse_mail(&id, &remote, &folder, raw, unread, starred)
        })
        .await??;
        output
            .send(MailSyncItem::Message(parsed))
            .await
            .context("Sync was cancelled")?;
    }
    Ok(())
}

/// Large messages are staged in plaintext only when the profile allows it.
#[cfg(feature = "staged-receive")]
fn staged_mode(plaintext_staging: bool) -> ReceiveMode {
    if plaintext_staging {
        ReceiveMode::Staged
    } else {
        ReceiveMode::Protected
    }
}

#[async_trait]
impl MailProvider for Imap {
    #[cfg(feature = "staged-receive")]
    async fn sync_staged(
        &self,
        account: &Account,
        password: &SecretString,
        known: &HashSet<String>,
        output: Sender<MailSyncItem>,
        plaintext_staging: bool,
    ) -> anyhow::Result<Vec<String>> {
        sync_imap_session_mode(
            imap(account, password).await?,
            account,
            known,
            output,
            None,
            staged_mode(plaintext_staging),
            None,
        )
        .await
    }
    #[cfg(feature = "condstore")]
    async fn sync_resuming(
        &self,
        account: &Account,
        password: &SecretString,
        known: &HashSet<String>,
        resume: &condstore::Resume,
        output: Sender<MailSyncItem>,
        plaintext_staging: bool,
    ) -> anyhow::Result<Vec<String>> {
        sync_imap_session_mode(
            imap(account, password).await?,
            account,
            known,
            output,
            None,
            staged_mode(plaintext_staging),
            Some(resume),
        )
        .await
    }
    async fn sync(
        &self,
        account: &Account,
        password: &SecretString,
        known: &HashSet<String>,
        output: Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        sync_imap_session(imap(account, password).await?, account, known, output, None).await
    }
    async fn move_mail(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
        folder: &str,
    ) -> anyhow::Result<Option<String>> {
        move_imap_session(imap(account, password).await?, mail, folder).await
    }

    async fn set_flags(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        flags_imap_session(imap(account, password).await?, mail, changes).await
    }
    async fn inspect_flags(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
    ) -> anyhow::Result<crate::mail_actions::Flags> {
        inspect_flags_imap_session(imap(account, password).await?, mail).await
    }
    async fn inspect_move(
        &self,
        account: &Account,
        password: &SecretString,
        receipt: &crate::mail_actions::MoveReceipt,
    ) -> anyhow::Result<Mail> {
        Ok(recovery::resolve(account, password, receipt).await?.summary)
    }
}

async fn inspect_flags_imap_session<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    mut session: async_imap::Session<T>,
    mail: &Mail,
) -> anyhow::Result<crate::mail_actions::Flags> {
    let mailbox = session.select(&mail.folder).await?;
    let uid = validate_uid(mail, mailbox.uid_validity)?;
    let numeric_uid = uid.parse::<u32>().context("Invalid message UID")?;
    let fetched = sync_queries::fetch(&mut session, &uid, "(UID FLAGS)").await?;
    let current = fetched
        .into_iter()
        .find(|entry| entry.uid == Some(numeric_uid))
        .context("The message is no longer present in this folder.")?;
    Ok(crate::mail_actions::Flags {
        unread: Some(current.unread),
        starred: Some(current.starred),
    })
}

async fn flags_imap_session<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    mut session: async_imap::Session<T>,
    mail: &Mail,
    changes: crate::mail_actions::Flags,
) -> anyhow::Result<()> {
    let mailbox = session.select(&mail.folder).await?;
    let uid = validate_uid(mail, mailbox.uid_validity)?;
    let mut acknowledged = false;
    for (flag, value) in [
        ("\\Seen", changes.unread.map(|v| !v)),
        ("\\Flagged", changes.starred),
    ] {
        if let Some(set) = value {
            let result = session
                .run_command_and_check_ok(format!(
                    "UID STORE {uid} {}FLAGS.SILENT ({flag})",
                    if set { "+" } else { "-" }
                ))
                .await;
            if let Err(error) = result {
                if !acknowledged
                    && matches!(
                        error,
                        async_imap::error::Error::No(_)
                            | async_imap::error::Error::Bad(_)
                            | async_imap::error::Error::Validate(_)
                    )
                {
                    return Err(crate::mail_actions::FlagsRejected(error.to_string()).into());
                }
                return Err(error.into());
            }
            acknowledged = true;
        }
    }
    // Acknowledged STORE is committed even if the connection closes on logout.
    let _ = session.logout().await;
    Ok(())
}

async fn move_imap_session<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    mut session: async_imap::Session<T>,
    mail: &Mail,
    folder: &str,
) -> anyhow::Result<Option<String>> {
    anyhow::ensure!(
        !folder.is_empty() && !folder.contains(['\r', '\n']),
        "Choose a valid folder."
    );
    let mailbox = session.select(&mail.folder).await?;
    let uid = validate_uid(mail, mailbox.uid_validity)?;
    if !session.capabilities().await?.has_str("MOVE") {
        return Err(crate::mail_actions::MoveRefused(
            "This IMAP server does not support safe MOVE. Move this message with your server's webmail.".into(),
        )
        .into());
    }
    let receipt = receipts::move_message(&mut session, &uid, folder).await?;
    // The tagged MOVE acknowledgment commits the action. A dropped connection
    // during logout must not retain the old source or invite a duplicate retry.
    let _ = session.logout().await;
    Ok(receipt)
}

/// The cached remote identity must match the selected mailbox's UIDVALIDITY.
pub fn validate_uid(mail: &Mail, validity: Option<u32>) -> anyhow::Result<String> {
    let (old, uid) = mail
        .remote_id
        .split_once('.')
        .context("Missing IMAP identity; sync this account again.")?;
    anyhow::ensure!(
        Some(old.parse::<u32>()?) == validity,
        "The mailbox identity changed. Sync before modifying messages."
    );
    uid.parse::<u32>()?;
    Ok(uid.to_string())
}

struct PopConnection<S>(BufReader<S>);
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> PopConnection<S> {
    async fn line(&mut self) -> anyhow::Result<String> {
        let mut data = Vec::new();
        let mut limited = (&mut self.0).take(64 * 1024);
        let read = limited.read_until(b'\n', &mut data);
        tokio::time::timeout(Duration::from_secs(30), read)
            .await
            .context("POP3 server timed out")??;
        anyhow::ensure!(
            !data.is_empty() && data.ends_with(b"\n"),
            "Invalid or oversized POP3 response."
        );
        Ok(String::from_utf8(data)?)
    }
    async fn command(&mut self, command: &str) -> anyhow::Result<String> {
        anyhow::ensure!(!command.contains(['\r', '\n']), "Invalid POP3 command.");
        self.0
            .get_mut()
            .write_all(format!("{command}\r\n").as_bytes())
            .await?;
        self.0.get_mut().flush().await?;
        let line = self.line().await?;
        anyhow::ensure!(line.starts_with("+OK"), "POP3 server rejected the request.");
        Ok(line)
    }
    async fn multiline(&mut self, limit: usize) -> anyhow::Result<Vec<u8>> {
        let mut data = Vec::new();
        loop {
            let mut line = Vec::new();
            let mut limited = (&mut self.0).take((limit - data.len() + 1) as u64);
            let read = limited.read_until(b'\n', &mut line);
            tokio::time::timeout(Duration::from_secs(30), read)
                .await
                .context("POP3 download timed out")??;
            anyhow::ensure!(!line.is_empty(), "POP3 server disconnected.");
            if line == b".\r\n" || line == b".\n" {
                break;
            }
            if line.starts_with(b"..") {
                line.remove(0);
            }
            anyhow::ensure!(
                data.len() + line.len() <= limit,
                "POP3 response exceeds the size limit."
            );
            data.extend(line);
        }
        Ok(data)
    }
}

async fn sync_pop_session<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    conn: PopConnection<S>,
    account: &Account,
    known: &HashSet<String>,
    output: Sender<MailSyncItem>,
) -> anyhow::Result<Vec<String>> {
    sync_pop_session_mode(conn, account, known, output, ReceiveMode::Legacy).await
}

async fn sync_pop_session_mode<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    mut conn: PopConnection<S>,
    account: &Account,
    known: &HashSet<String>,
    output: Sender<MailSyncItem>,
    mode: ReceiveMode,
) -> anyhow::Result<Vec<String>> {
    let mut deferred = Vec::new();
    conn.command("UIDL")
        .await
        .context("A POP3 server with stable UIDL identifiers is required")?;
    let listing = String::from_utf8(conn.multiline(8 * 1024 * 1024).await?)?;
    output
        .send(MailSyncItem::InboxSyncStarted {
            account: account.id.clone(),
            epoch: "pop3".into(),
        })
        .await?;
    for line in listing.lines() {
        let (number, uid) = line
            .split_once(' ')
            .context("Invalid POP3 message listing")?;
        let number = number.parse::<u32>()?;
        let uid = uid.trim().to_string();
        anyhow::ensure!(
            number > 0 && !uid.is_empty() && !uid.chars().any(char::is_whitespace),
            "Invalid POP3 message identity"
        );
        if known.contains(&format!("{}:INBOX:{uid}", account.id)) {
            continue;
        }
        let size = conn.command(&format!("LIST {number}")).await?;
        let size = size
            .split_whitespace()
            .nth(2)
            .and_then(|s| s.parse::<usize>().ok())
            .context("Invalid POP3 message size")?;
        if size > MAX_MESSAGE_BYTES {
            if mode != ReceiveMode::Legacy {
                deferred.push((number, uid, size));
            } else {
                output.send(MailSyncItem::SkippedLarge).await?;
            }
            continue;
        }
        conn.command(&format!("RETR {number}")).await?;
        let raw = conn.multiline(MAX_MESSAGE_BYTES).await?;
        let id = account.id.clone();
        let parsed =
            tokio::task::spawn_blocking(move || parse_mail(&id, &uid, "INBOX", raw, true, false))
                .await??;
        output
            .send(MailSyncItem::Message(parsed))
            .await
            .context("Sync was cancelled")?;
    }
    anyhow::ensure!(
        mode != ReceiveMode::Protected || deferred.is_empty(),
        "Large-message downloads need encrypted staging for this encrypted profile. Smaller messages were downloaded; the originals remain on the server."
    );
    #[cfg(feature = "staged-receive")]
    for (number, uid, size) in deferred {
        conn.command(&format!("RETR {number}")).await?;
        let source = tempfile::NamedTempFile::new()?;
        let mut writer = tokio::fs::File::from_std(source.reopen()?);
        streaming::pop_body(&mut conn.0, size as u64, &mut writer, Some(&output)).await?;
        drop(writer);
        let id = account.id.clone();
        let message = tokio::task::spawn_blocking(move || {
            staging::prepare(source, &id, &uid, "INBOX", true, false)
        })
        .await??;
        output
            .send(MailSyncItem::StagedMessage(message))
            .await
            .context("Sync was cancelled")?;
    }
    // Always leave originals on the POP3 server. Folders and flags are local.
    output
        .send(MailSyncItem::InboxSyncFinished {
            account: account.id.clone(),
            epoch: "pop3".into(),
        })
        .await?;
    conn.command("QUIT").await?;
    Ok(vec!["INBOX".into()])
}

#[async_trait]
impl MailProvider for Pop3 {
    #[cfg(feature = "staged-receive")]
    async fn sync_staged(
        &self,
        account: &Account,
        password: &SecretString,
        known: &HashSet<String>,
        output: Sender<MailSyncItem>,
        plaintext_staging: bool,
    ) -> anyhow::Result<Vec<String>> {
        let mode = staged_mode(plaintext_staging);
        sync_pop_session_mode(pop(account, password).await?, account, known, output, mode).await
    }
    async fn sync(
        &self,
        account: &Account,
        password: &SecretString,
        known: &HashSet<String>,
        output: Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        sync_pop_session(pop(account, password).await?, account, known, output).await
    }

    async fn move_mail(
        &self,
        _: &Account,
        _: &SecretString,
        _: &Mail,
        _: &str,
    ) -> anyhow::Result<Option<String>> {
        Ok(None)
    }
    async fn set_flags(
        &self,
        _: &Account,
        _: &SecretString,
        _: &Mail,
        _: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

pub async fn send(
    account: &Account,
    password: &SecretString,
    message: lettre::Message,
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        !account.smtp_host.trim().is_empty(),
        "Add an SMTP server to this account before sending."
    );
    let transport = smtp_transport(account, password)?;
    deliver(transport, message).await
}

async fn deliver(
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    message: lettre::Message,
) -> anyhow::Result<Vec<u8>> {
    let raw = message.formatted();
    deliver_raw(transport, message.envelope(), &raw).await?;
    Ok(raw)
}

#[derive(Debug, Clone)]
pub enum DeliveryFailure {
    Rejected(String),
    Uncertain,
}

impl std::fmt::Display for DeliveryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(reason)=>write!(f,"Message was not sent. Your draft has been kept. {reason}"),
            Self::Uncertain=>f.write_str("SMTP could not confirm delivery. Review this message in Outbox before trying again."),
        }
    }
}
impl std::error::Error for DeliveryFailure {}

pub async fn send_raw(
    account: &Account,
    password: &SecretString,
    envelope: &lettre::address::Envelope,
    raw: &[u8],
) -> Result<(), DeliveryFailure> {
    let transport = smtp_transport(account, password).map_err(|_| {
        DeliveryFailure::Rejected("Check your SMTP server and security settings.".into())
    })?;
    deliver_raw(transport, envelope, raw).await
}
async fn deliver_raw(
    transport: lettre::AsyncSmtpTransport<lettre::Tokio1Executor>,
    envelope: &lettre::address::Envelope,
    raw: &[u8],
) -> Result<(), DeliveryFailure> {
    use lettre::AsyncTransport;
    if raw.is_empty() || raw.len() > MAX_MESSAGE_BYTES {
        return Err(DeliveryFailure::Rejected(
            "The message exceeds the sending size limit.".into(),
        ));
    }
    transport.send_raw(envelope, raw).await.map_err(|error| {
        if let Some(code) = error.status() {
            DeliveryFailure::Rejected(format!("The SMTP server rejected it (status {code})."))
        } else if error.is_tls() || error.is_client() || error.is_transport_shutdown() {
            DeliveryFailure::Rejected(
                "Check the SMTP connection and authentication settings.".into(),
            )
        } else {
            DeliveryFailure::Uncertain
        }
    })?;
    Ok(())
}

/// Read-only handshake/authentication probe. Never sends mail or modifies messages.
pub async fn test_incoming(account: &Account, password: &SecretString) -> anyhow::Result<String> {
    tokio::time::timeout(Duration::from_secs(35), async {
        match account.protocol {
            Protocol::Imap => {
                let mut session = imap(account, password).await?;
                let mailbox = session.examine("INBOX").await?;
                let count = mailbox.exists;
                session.logout().await?;
                Ok(format!(
                    "IMAP connection and authentication succeeded; {count} messages in Inbox."
                ))
            }
            Protocol::Pop3 => {
                let mut connection = pop(account, password).await?;
                connection.command("STAT").await?;
                connection.command("QUIT").await?;
                Ok("POP3 connection and authentication succeeded.".into())
            }
        }
    })
    .await
    .context("Connection test timed out after 35 seconds")?
}

fn smtp_transport(
    account: &Account,
    password: &SecretString,
) -> anyhow::Result<lettre::AsyncSmtpTransport<lettre::Tokio1Executor>> {
    use lettre::{
        AsyncSmtpTransport, Tokio1Executor,
        transport::smtp::authentication::{Credentials, Mechanism},
    };
    anyhow::ensure!(!account.smtp_host.is_empty(), "Enter an SMTP hostname");
    let mut transport = if account.smtp_security() == ConnectionSecurity::Tls {
        AsyncSmtpTransport::<Tokio1Executor>::relay(&account.smtp_host)?
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&account.smtp_host)?
    }
    .port(account.smtp_port)
    .timeout(Some(Duration::from_secs(30)));
    if account.smtp_auth != SmtpAuth::None {
        transport = transport.credentials(Credentials::new(
            account.smtp_username().into(),
            password.expose_secret().into(),
        ));
        transport = match account.smtp_auth {
            SmtpAuth::Plain => transport.authentication(vec![Mechanism::Plain]),
            SmtpAuth::Login => transport.authentication(vec![Mechanism::Login]),
            _ => transport,
        };
    }
    Ok(transport.build())
}
pub async fn test_smtp(account: &Account, password: &SecretString) -> anyhow::Result<String> {
    let success = tokio::time::timeout(
        Duration::from_secs(35),
        smtp_transport(account, password)?.test_connection(),
    )
    .await
    .context("SMTP connection test timed out")??;
    anyhow::ensure!(success, "SMTP server did not accept the connection test");
    Ok("SMTP connection and authentication succeeded. No email was sent.".into())
}

/// Validate safe, exact source removal before writing to another account.
pub async fn prepare_transfer(
    account: &Account,
    secret: &SecretString,
    mail: &Mail,
) -> anyhow::Result<()> {
    let mut session = imap(account, secret).await?;
    validate_uid(mail, session.select(&mail.folder).await?.uid_validity)?;
    anyhow::ensure!(
        session.capabilities().await?.has_str("UIDPLUS"),
        "The source server needs UIDPLUS to move between accounts safely."
    );
    session.logout().await?;
    Ok(())
}
pub async fn append_transfer(
    account: &Account,
    secret: &SecretString,
    mail: &Mail,
    folder: &str,
    raw: Vec<u8>,
) -> anyhow::Result<Option<String>> {
    anyhow::ensure!(
        !folder.is_empty() && !folder.contains(['\r', '\n']),
        "Choose a valid destination folder."
    );
    let mut session = imap(account, secret).await?;
    let receipt = receipts::append_message(&mut session, mail, folder, &raw).await?;
    let _ = session.logout().await;
    Ok(receipt)
}
pub async fn finish_transfer(
    account: &Account,
    secret: &SecretString,
    mail: &Mail,
) -> anyhow::Result<()> {
    finish_transfer_session(imap(account, secret).await?, mail).await
}

async fn finish_transfer_session<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    mut session: async_imap::Session<T>,
    mail: &Mail,
) -> anyhow::Result<()> {
    finish_transfer_commands(&mut session, mail).await?;
    let _ = session.logout().await;
    Ok(())
}

/// Revalidate and remove exactly the original UID on an open session; the caller
/// owns the session lifetime so an interrupted move can reuse it.
pub async fn finish_transfer_commands<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    session: &mut async_imap::Session<T>,
    mail: &Mail,
) -> anyhow::Result<()> {
    let uid = validate_uid(mail, session.select(&mail.folder).await?.uid_validity)?;
    anyhow::ensure!(
        session.capabilities().await?.has_str("UIDPLUS"),
        "Safe removal requires UIDPLUS."
    );
    session
        .run_command_and_check_ok(format!("UID STORE {uid} +FLAGS.SILENT (\\Deleted)"))
        .await?;
    session
        .run_command_and_check_ok(format!("UID EXPUNGE {uid}"))
        .await?;
    Ok(())
}

/// Bounded diagnostic using the production sync path, restricted to Inbox.
#[cfg(feature = "test-support")]
pub async fn sync_inbox(
    account: &Account,
    secret: &SecretString,
    known: &HashSet<String>,
    output: Sender<MailSyncItem>,
) -> anyhow::Result<Vec<String>> {
    sync_imap_session(
        imap(account, secret).await?,
        account,
        known,
        output,
        Some("INBOX"),
    )
    .await
}

#[cfg(test)]
#[path = "smtp_tests.rs"]
mod smtp_tests;
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transfer_cleanup_requires_store_and_expunge_acknowledgments() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        for rejected in [None, Some("STORE"), Some("EXPUNGE")] {
            let (client, server) = tokio::io::duplex(4096);
            let server = tokio::spawn(async move {
                let mut server = BufReader::new(server);
                let mut mutations = Vec::new();
                loop {
                    let mut line = String::new();
                    if server.read_line(&mut line).await.unwrap() == 0 {
                        break;
                    }
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    if command == "LOGOUT" {
                        break;
                    }
                    let response = if command.starts_with("SELECT ") {
                        "* 1 EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n"
                    } else if command == "CAPABILITY" {
                        "* CAPABILITY IMAP4rev1 UIDPLUS\r\n"
                    } else {
                        ""
                    };
                    let operation = if command == "UID STORE 7 +FLAGS.SILENT (\\Deleted)" {
                        Some("STORE")
                    } else if command == "UID EXPUNGE 7" {
                        Some("EXPUNGE")
                    } else {
                        None
                    };
                    if let Some(operation) = operation {
                        mutations.push(operation);
                    }
                    let failed = operation.is_some() && rejected == operation;
                    let status = if failed {
                        "NO not permitted"
                    } else {
                        "OK done"
                    };
                    server
                        .get_mut()
                        .write_all(format!("{response}{tag} {status}\r\n").as_bytes())
                        .await
                        .unwrap();
                    if failed {
                        break;
                    }
                }
                assert_eq!(
                    mutations,
                    if rejected == Some("STORE") {
                        vec!["STORE"]
                    } else {
                        vec!["STORE", "EXPUNGE"]
                    }
                );
            });
            let session = async_imap::Client::new(client)
                .login("fixture", "secret")
                .await
                .unwrap();
            let mail = parse_mail(
                "fixture",
                "42.7",
                "INBOX",
                b"From: fixture@example.test\r\nSubject: Transfer\r\n\r\nBody".to_vec(),
                true,
                false,
            )
            .unwrap();
            assert_eq!(
                finish_transfer_session(session, &mail.summary)
                    .await
                    .is_err(),
                rejected.is_some()
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn imap_read_unread_and_flag_actions_emit_only_the_requested_store_and_honor_failure() {
        use crate::mail_actions::Flags;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        for (changes, expected, reject) in [
            (
                Flags {
                    unread: Some(false),
                    starred: None,
                },
                "UID STORE 7 +FLAGS.SILENT (\\Seen)",
                false,
            ),
            (
                Flags {
                    unread: Some(true),
                    starred: None,
                },
                "UID STORE 7 -FLAGS.SILENT (\\Seen)",
                false,
            ),
            (
                Flags {
                    unread: None,
                    starred: Some(true),
                },
                "UID STORE 7 +FLAGS.SILENT (\\Flagged)",
                false,
            ),
            (
                Flags {
                    unread: None,
                    starred: Some(false),
                },
                "UID STORE 7 -FLAGS.SILENT (\\Flagged)",
                false,
            ),
            (
                Flags {
                    unread: Some(false),
                    starred: None,
                },
                "UID STORE 7 +FLAGS.SILENT (\\Seen)",
                true,
            ),
        ] {
            let (client, server) = tokio::io::duplex(4096);
            let server = tokio::spawn(async move {
                let mut server = BufReader::new(server);
                let mut stores = 0;
                loop {
                    let mut line = String::new();
                    if server.read_line(&mut line).await.unwrap() == 0 {
                        break;
                    }
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    if command == "LOGOUT" {
                        break;
                    } // Ack remains valid on disconnect.
                    let response = if command.starts_with("SELECT ") {
                        "* 1 EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n"
                    } else {
                        ""
                    };
                    let status = if command.starts_with("UID STORE ") {
                        stores += 1;
                        assert_eq!(command, expected);
                        if reject {
                            "NO not permitted"
                        } else {
                            "OK stored"
                        }
                    } else {
                        "OK completed"
                    };
                    server
                        .get_mut()
                        .write_all(format!("{response}{tag} {status}\r\n").as_bytes())
                        .await
                        .unwrap();
                    if reject && stores == 1 {
                        break;
                    }
                }
                assert_eq!(stores, 1);
            });
            let session = async_imap::Client::new(client)
                .login("fixture", "secret")
                .await
                .unwrap();
            let mail = parse_mail(
                "fixture",
                "42.7",
                "INBOX",
                b"From: fixture@example.test\r\nSubject: Actions\r\n\r\nBody".to_vec(),
                true,
                false,
            )
            .unwrap();
            let result = flags_imap_session(session, &mail.summary, changes).await;
            assert_eq!(result.is_err(), reject);
            if let Err(error) = result {
                assert!(error.is::<crate::mail_actions::FlagsRejected>());
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn imap_flag_refusal_requires_no_prior_acknowledgement() {
        use crate::mail_actions::{Flags, FlagsRejected};
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        for (first_status, second_status, rejected) in [
            (Some("NO refused"), None, true),
            (Some("BAD invalid"), None, true),
            (None, None, false),
            (Some("OK stored"), Some("NO refused"), false),
            (Some("OK stored"), None, false),
        ] {
            let (client, server) = tokio::io::duplex(4096);
            let server = tokio::spawn(async move {
                let mut server = BufReader::new(server);
                let mut stores = 0;
                loop {
                    let mut line = String::new();
                    if server.read_line(&mut line).await.expect("command") == 0 {
                        break;
                    }
                    let (tag, command) = line.trim_end().split_once(' ').expect("tag");
                    let status = if command.starts_with("UID STORE ") {
                        stores += 1;
                        let status = if stores == 1 {
                            first_status
                        } else {
                            second_status
                        };
                        let Some(status) = status else { break };
                        status
                    } else {
                        "OK completed"
                    };
                    let prefix = if command.starts_with("SELECT ") {
                        "* 1 EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n"
                    } else {
                        ""
                    };
                    server
                        .get_mut()
                        .write_all(format!("{prefix}{tag} {status}\r\n").as_bytes())
                        .await
                        .expect("response");
                    if stores > 0 && !status.starts_with("OK") {
                        break;
                    }
                }
                assert!((1..=2).contains(&stores));
            });
            let session = async_imap::Client::new(client)
                .login("fixture", "secret")
                .await
                .expect("login");
            let mail = parse_mail(
                "fixture",
                "42.7",
                "INBOX",
                b"From: fixture@example.test\r\nSubject: Flags\r\n\r\nBody".to_vec(),
                true,
                false,
            )
            .expect("message");
            let error = flags_imap_session(
                session,
                &mail.summary,
                Flags {
                    unread: Some(false),
                    starred: Some(true),
                },
            )
            .await
            .expect_err("failed STORE");
            assert_eq!(error.is::<FlagsRejected>(), rejected, "{error:#}");
            assert_eq!(
                crate::mail_actions::classify_move_failure(&error),
                crate::mail_actions::MoveFailure::Uncertain,
                "flag refusal must not authorise a device-only move"
            );
            server.await.expect("mock server");
        }
    }

    #[tokio::test]
    async fn move_from_spaced_folder_to_inbox_commits_before_logout_disconnect() {
        let (client, server) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            for expected in [
                "LOGIN \"fixture\" \"secret\"",
                "SELECT \"A. Keep folder\"",
                "CAPABILITY",
                "UID MOVE 7 \"INBOX\"",
                "LOGOUT",
            ] {
                let mut line = String::new();
                server.read_line(&mut line).await.unwrap();
                let (tag, command) = line.trim_end().split_once(' ').unwrap();
                assert_eq!(command, expected);
                if command == "LOGOUT" {
                    break;
                }
                let response = if command.starts_with("SELECT") {
                    "* 1 EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n"
                } else if command == "CAPABILITY" {
                    "* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n"
                } else {
                    ""
                };
                server
                    .get_mut()
                    .write_all(format!("{response}{tag} OK completed\r\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        let session = async_imap::Client::new(client)
            .login("fixture", "secret")
            .await
            .unwrap();
        let mail = parse_mail(
            "fixture",
            "42.7",
            "A. Keep folder",
            b"From: fixture@example.test\r\nSubject: Move fixture\r\n\r\nTest".to_vec(),
            true,
            false,
        )
        .unwrap();
        move_imap_session(session, &mail.summary, "INBOX")
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn imap_sync_uses_valid_fetch_lists_and_batches_bodies() {
        let raw = b"From: Test <test@example.com>\r\nSubject: Wire regression\r\n\r\nBody";
        let (client, server) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            for stage in 0..8 {
                let mut line = String::new();
                server.read_line(&mut line).await.unwrap();
                let (tag, command) = line.trim_end().split_once(' ').unwrap();
                let response = match stage {
                    0 => {
                        assert!(command.starts_with("LOGIN "));
                        String::new()
                    }
                    1 => {
                        assert_eq!(command, "CAPABILITY");
                        "* CAPABILITY IMAP4rev1\r\n".into()
                    }
                    2 => {
                        assert_eq!(command, "LIST \"\" *");
                        "* LIST () \"/\" \"INBOX\"\r\n".into()
                    }
                    3 => {
                        assert_eq!(command, "SELECT \"INBOX\"");
                        "* 2 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n".into()
                    }
                    4 => {
                        assert_eq!(command, "UID SEARCH ALL");
                        "* SEARCH 7 8\r\n".into()
                    }
                    5 => {
                        assert_eq!(command, "UID FETCH 8,7 (UID FLAGS RFC822.SIZE)");
                        format!(
                            "* 1 FETCH (UID 8 FLAGS () RFC822.SIZE {})\r\n* 2 FETCH (UID 7 FLAGS (\\Seen \\Flagged) RFC822.SIZE {})\r\n",
                            raw.len(),
                            raw.len()
                        )
                    }
                    6 => {
                        assert_eq!(command, "UID FETCH 8,7 (UID FLAGS BODY.PEEK[])");
                        format!(
                            "* 1 FETCH (UID 8 FLAGS () BODY[] {{{}}}\r\n{})\r\n* 2 FETCH (UID 7 FLAGS (\\Seen \\Flagged) BODY[] {{{}}}\r\n{})\r\n",
                            raw.len(),
                            String::from_utf8_lossy(raw),
                            raw.len(),
                            String::from_utf8_lossy(raw)
                        )
                    }
                    _ => {
                        assert_eq!(command, "LOGOUT");
                        "* BYE goodbye\r\n".into()
                    }
                };
                server
                    .get_mut()
                    .write_all(format!("{response}{tag} OK done\r\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        let session = async_imap::Client::new(client)
            .login("test", "secret")
            .await
            .unwrap();
        let account: Account = serde_json::from_value(serde_json::json!({"id":"test", "name":"Test", "email":"test@example.com", "protocol":"Imap", "host":"localhost", "port":993, "username":"test", "smtp_host":"localhost", "smtp_port":465})).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let folders = tokio::time::timeout(
            Duration::from_secs(5),
            sync_imap_session(session, &account, &HashSet::new(), tx, None),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(folders, ["INBOX"]);
        let mut messages = Vec::new();
        while let Some(item) = rx.recv().await {
            if let MailSyncItem::Message(mail) = item {
                messages.push(mail);
            }
        }
        assert_eq!(messages.len(), 2);
        assert!(messages[0].summary.unread);
        assert!(messages[1].summary.starred);
        assert_eq!(messages[1].raw, raw);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pop3_wire_commands_dot_stuffing_and_binary_bodies() {
        let (client, server) = tokio::io::duplex(4096);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            for command in [
                "USER sam\r\n",
                "PASS test-password\r\n",
                "RETR 1\r\n",
                "QUIT\r\n",
            ] {
                let mut line = String::new();
                server.read_line(&mut line).await.unwrap();
                assert_eq!(line, command);
                server.get_mut().write_all(b"+OK\r\n").await.unwrap();
                if command.starts_with("RETR") {
                    server
                        .get_mut()
                        .write_all(
                            b"Subject: A message\r\n\r\n..dot-stuffed\r\nraw \xff byte\r\n.\r\n",
                        )
                        .await
                        .unwrap();
                }
            }
        });
        let mut connection = PopConnection(BufReader::new(client));
        connection.command("USER sam").await.unwrap();
        connection.command("PASS test-password").await.unwrap();
        connection.command("RETR 1").await.unwrap();
        let body = connection.multiline(4096).await.unwrap();
        assert!(
            body.windows(b".dot-stuffed\r".len())
                .any(|b| b == b".dot-stuffed\r")
        );
        assert!(body.contains(&255));
        connection.command("QUIT").await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pop3_rejects_injection_oversized_and_truncated_responses() {
        let (client, mut server) = tokio::io::duplex(128);
        let mut connection = PopConnection(BufReader::new(client));
        assert!(connection.command("USER a\r\nDELE 1").await.is_err());
        server.write_all(b"too much data\r\n.\r\n").await.unwrap();
        assert!(connection.multiline(5).await.is_err());
        let (client, mut server) = tokio::io::duplex(128);
        server.write_all(b"unterminated").await.unwrap();
        drop(server);
        assert!(PopConnection(BufReader::new(client)).line().await.is_err());
    }

    #[test]
    fn imap_uidvalidity_must_match_before_mutating_mail() {
        let mail = parse_mail(
            "account",
            "12.34",
            "INBOX",
            b"Subject: Test\r\n\r\nBody".to_vec(),
            true,
            false,
        )
        .unwrap()
        .summary;
        assert_eq!(validate_uid(&mail, Some(12)).unwrap(), "34");
        assert!(validate_uid(&mail, Some(13)).is_err());
        assert!(validate_uid(&mail, None).is_err());
    }
}
