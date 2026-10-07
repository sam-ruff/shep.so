//! Permitted remote images for the confined reader. Only URLs discovered in a
//! cached message are fetched, after the shared rules allow that message.
//! Converted bytes stay in a bounded memory cache that revocation clears.
use crate::{api::MobileProfile, operations::stored_mail};
use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use shep_mail_core::{
    model::ImagePolicy,
    remote_images::{self, ReqwestTransport, Rules, Transport, Webp},
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};
use tokio::sync::Semaphore;

/// Image keys per request; the reader asks for further batches as each lands.
pub(crate) const BATCH: usize = 8;
const MAX_ENTRIES: usize = 128;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_RULES: usize = 1000;

/// The synced policy and this device's exceptions, sent with each request.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ImageRules {
    policy: ImagePolicy,
    messages: Vec<String>,
    senders: Vec<String>,
    domains: Vec<String>,
    contacts: Vec<String>,
}

impl ImageRules {
    /// Moved messages keep exceptions saved under their earlier IDs.
    pub(crate) fn allows(&self, ids: &[&str], sender: &str) -> Result<bool> {
        ensure!(
            [&self.messages, &self.senders, &self.domains, &self.contacts]
                .iter()
                .all(|list| list.len() <= MAX_RULES && list.iter().all(|value| value.len() <= 512)),
            "Image exceptions are too large. Clear some in Preferences and retry."
        );
        let rules = Rules {
            policy: self.policy,
            messages: &self.messages,
            senders: &self.senders,
            domains: &self.domains,
            contacts: &self.contacts,
        };
        Ok(ids.iter().any(|id| rules.allows(id, sender)))
    }
}

#[derive(Default)]
struct Cache {
    epoch: u64,
    entries: HashMap<String, Arc<Webp>>,
    order: VecDeque<String>,
    bytes: usize,
}

impl Cache {
    fn get(&mut self, url: &str) -> Option<Arc<Webp>> {
        let image = self.entries.get(url)?.clone();
        self.order.retain(|value| value != url);
        self.order.push_back(url.to_owned());
        Some(image)
    }

    fn insert(&mut self, epoch: u64, url: String, image: Arc<Webp>) {
        if epoch != self.epoch || image.bytes.len() > MAX_BYTES || self.entries.contains_key(&url) {
            return;
        }
        self.bytes += image.bytes.len();
        self.entries.insert(url.clone(), image);
        self.order.push_back(url);
        while self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.bytes -= evicted.bytes.len();
            }
        }
    }

    fn forget(&mut self) {
        self.epoch += 1;
        self.entries.clear();
        self.order.clear();
        self.bytes = 0;
    }
}

pub(crate) struct Runtime {
    transport: Mutex<Arc<dyn Transport>>,
    requests: Arc<Semaphore>,
    fetches: Arc<Semaphore>,
    cache: Arc<Mutex<Cache>>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            transport: Mutex::new(Arc::new(ReqwestTransport::new())),
            requests: Arc::new(Semaphore::new(4)),
            fetches: Arc::new(Semaphore::new(4)),
            cache: Arc::default(),
        }
    }
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Runtime {
    #[cfg(test)]
    pub(crate) fn set_transport(&self, transport: Arc<dyn Transport>) {
        *lock(&self.transport) = transport;
    }

    /// Revocation: cached pixels go, and fetches already running cannot
    /// repopulate the cache or return their bytes.
    pub(crate) fn forget(&self) {
        lock(&self.cache).forget();
    }

    /// Cached bytes for an allowed message; never a network request.
    pub(crate) fn cached(
        &self,
        allowed: bool,
    ) -> impl Fn(&str) -> Option<Vec<u8>> + Send + 'static {
        let cache = self.cache.clone();
        move |url| {
            allowed
                .then(|| lock(&cache).get(url))
                .flatten()
                .map(|image| image.bytes.clone())
        }
    }
}

fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) async fn load(
    profile: &MobileProfile,
    id: String,
    keys: Vec<String>,
    rules: ImageRules,
) -> Result<Value> {
    ensure!(
        !keys.is_empty() && keys.len() <= BATCH && keys.iter().all(|key| valid_key(key)),
        "Invalid image request. Reopen the message and retry."
    );
    let runtime = &profile.operations.remote_images;
    let _admitted = runtime
        .requests
        .clone()
        .try_acquire_owned()
        .context("Images are still loading for other messages. Retry shortly.")?;
    let epoch = lock(&runtime.cache).epoch;
    let requested = id.clone();
    let (mail, raw) = profile
        .database
        .read(move |db| {
            let mail = stored_mail(db, &id)?;
            let raw: Vec<u8> =
                db.query_row("SELECT raw FROM mail WHERE id=?1", [&mail.id], |r| r.get(0))?;
            Ok((mail, raw))
        })
        .await?;
    ensure!(
        rules.allows(&[mail.id.as_str(), requested.as_str()], &mail.sender)?,
        "Remote images are blocked for this message. Choose Load images to allow them."
    );
    let discovered =
        tokio::task::spawn_blocking(move || shep_mail_core::document::remote_images(&raw))
            .await
            .context("Could not read this message's images. Retry.")?
            .context("Could not read this message's images. Retry.")?;
    let mut tasks = tokio::task::JoinSet::new();
    for key in keys {
        let url = discovered
            .iter()
            .find(|image| image.key == key)
            .map(|image| image.url.clone())
            .context("This image is not part of the message. Reopen it and retry.")?;
        let cache = runtime.cache.clone();
        let fetches = runtime.fetches.clone();
        let transport = lock(&runtime.transport).clone();
        tasks.spawn(async move {
            if let Some(image) = lock(&cache).get(&url) {
                return (key, Ok(image));
            }
            let _permit = fetches.acquire_owned().await;
            if let Some(image) = lock(&cache).get(&url) {
                return (key, Ok(image));
            }
            let result = remote_images::fetch(transport.as_ref(), &url)
                .await
                .map(Arc::new);
            if let Ok(image) = &result {
                lock(&cache).insert(epoch, url, image.clone());
            }
            (key, result)
        });
    }
    let (mut images, mut failed) = (Map::new(), Map::new());
    while let Some(joined) = tasks.join_next().await {
        let (key, result) = joined.context("An image download stopped unexpectedly. Retry.")?;
        match result {
            Ok(image) => {
                images.insert(
                    key,
                    json!({
                        "bytes": base64::engine::general_purpose::STANDARD.encode(&image.bytes),
                        "width": image.width,
                        "height": image.height,
                    }),
                );
            }
            Err(error) => {
                failed.insert(key, error.to_string().into());
            }
        }
    }
    ensure!(
        lock(&runtime.cache).epoch == epoch,
        "Image permission changed while loading. Reopen the message to continue."
    );
    Ok(json!({"images": images, "failed": failed}))
}
