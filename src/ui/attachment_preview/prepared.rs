//! Attachment previews prepared before they are asked for: the open message's
//! attachments and those of the nearest unread messages. Only metadata and
//! finished previews live here; preparation runs on its own engine lane.
use crate::{attachment_files::Preview, model::Mail};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

const BYTES: usize = 64 * 1024 * 1024;
const ENTRIES: usize = 32;
/// Unread messages near the selection whose attachments are prepared.
pub const NEARBY: usize = 2;
/// Rows either side of the selection that count as near.
pub const REACH: usize = 6;
/// Attachments prepared per message.
pub const PER_MESSAGE: usize = 8;
/// Failed or skipped attempts remembered so they are not retried on every
/// selection.
const ATTEMPT_MEMORY: usize = 256;

pub type Key = (String, usize);

#[derive(Default)]
pub struct Prepared {
    /// Most recently used first.
    entries: VecDeque<(Key, Arc<Preview>)>,
    bytes: usize,
    pending: HashSet<Key>,
    attempted: HashSet<Key>,
}

impl Prepared {
    pub fn get(&mut self, source: &str, index: usize) -> Option<Arc<Preview>> {
        let position = self
            .entries
            .iter()
            .position(|((id, i), _)| id == source && *i == index)?;
        let entry = self.entries.remove(position)?;
        let preview = entry.1.clone();
        self.entries.push_front(entry);
        Some(preview)
    }

    /// Indices that are neither prepared, in flight nor already attempted.
    pub fn wanted(&self, source: &str, count: usize) -> Vec<usize> {
        (0..count.min(PER_MESSAGE))
            .filter(|index| {
                let key = (source.to_owned(), *index);
                !self.pending.contains(&key)
                    && !self.attempted.contains(&key)
                    && !self.entries.iter().any(|(k, _)| *k == key)
            })
            .collect()
    }

    pub fn requested(&mut self, source: &str, indices: &[usize]) {
        self.pending
            .extend(indices.iter().map(|index| (source.to_owned(), *index)));
    }

    /// Forgets requests the full queue refused, so a later selection retries.
    pub fn abandoned(&mut self, source: &str, indices: &[usize]) {
        for index in indices {
            self.pending.remove(&(source.to_owned(), *index));
        }
    }

    /// Stores a finished preview. Entries for `protect` (the open message) are
    /// evicted last.
    pub fn finished(
        &mut self,
        source: String,
        index: usize,
        preview: Option<Arc<Preview>>,
        protect: Option<&str>,
    ) {
        let key = (source, index);
        self.pending.remove(&key);
        let Some(preview) = preview else {
            if self.attempted.len() >= ATTEMPT_MEMORY {
                self.attempted.clear();
            }
            self.attempted.insert(key);
            return;
        };
        let weight = preview.weight();
        if weight > BYTES {
            return;
        }
        if let Some(position) = self.entries.iter().position(|(k, _)| *k == key)
            && let Some((_, old)) = self.entries.remove(position)
        {
            self.bytes -= old.weight();
        }
        self.bytes += weight;
        self.entries.push_front((key, preview));
        self.evict(protect);
    }

    fn evict(&mut self, protect: Option<&str>) {
        while self.bytes > BYTES || self.entries.len() > ENTRIES {
            let victim = self
                .entries
                .iter()
                .rposition(|((id, _), _)| Some(id.as_str()) != protect)
                .unwrap_or(self.entries.len() - 1);
            let Some((_, preview)) = self.entries.remove(victim) else {
                return;
            };
            self.bytes -= preview.weight();
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn keys(&self) -> Vec<Key> {
        self.entries.iter().map(|(key, _)| key.clone()).collect()
    }

    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

/// The nearest unread messages with attachments, checked outward from the
/// selection and preferring the next row at equal distance.
pub fn nearby<'a>(rows: &'a [Mail], selected: &str) -> Vec<&'a Mail> {
    let Some(position) = rows.iter().position(|mail| mail.id == selected) else {
        return Vec::new();
    };
    (1..=REACH)
        .flat_map(|distance| {
            [
                rows.get(position + distance),
                position.checked_sub(distance).and_then(|i| rows.get(i)),
            ]
        })
        .flatten()
        .filter(|mail| mail.unread && mail.attachment_count > 0)
        .take(NEARBY)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachment_files::Content;

    fn preview(bytes: usize) -> Arc<Preview> {
        Arc::new(Preview {
            name: String::new(),
            size: bytes,
            kind: "Text",
            content: Content::Text {
                text: "x".repeat(bytes),
                truncated: false,
            },
            refusal: None,
        })
    }

    fn mail(id: &str, unread: bool, attachments: usize) -> Mail {
        Mail {
            id: id.into(),
            account_id: String::new(),
            remote_id: String::new(),
            folder: "INBOX".into(),
            sender: String::new(),
            recipient: String::new(),
            subject: String::new(),
            preview: String::new(),
            timestamp: 0,
            unread,
            starred: false,
            attachment_count: attachments,
        }
    }

    #[test]
    fn nearby_picks_the_closest_unread_messages_with_attachments() {
        let rows = [
            mail("a", true, 1),
            mail("b", false, 3),
            mail("c", true, 0),
            mail("selected", true, 2),
            mail("d", false, 1),
            mail("e", true, 1),
            mail("f", false, 1),
            mail("g", true, 1),
        ];
        let ids: Vec<_> = nearby(&rows, "selected")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        // "e" is two rows below and "a" three above; "g" is beyond the limit.
        assert_eq!(ids, ["e", "a"]);
        assert!(nearby(&rows, "missing").is_empty());
    }

    #[test]
    fn nearby_ignores_rows_out_of_reach() {
        let mut rows = vec![mail("selected", false, 0)];
        rows.extend((0..REACH).map(|i| mail(&i.to_string(), false, 0)));
        rows.push(mail("far", true, 1));
        assert!(nearby(&rows, "selected").is_empty());
    }

    #[test]
    fn wanted_skips_prepared_pending_and_failed_attachments() {
        let mut prepared = Prepared::default();
        prepared.finished("m".into(), 0, Some(preview(10)), None);
        prepared.requested("m", &[1]);
        prepared.finished("m".into(), 2, None, None);
        assert_eq!(prepared.wanted("m", 4), [3]);
        assert_eq!(prepared.wanted("other", 20).len(), PER_MESSAGE);
        prepared.abandoned("m", &[1]);
        assert_eq!(prepared.wanted("m", 4), [1, 3]);
    }

    #[test]
    fn eviction_keeps_the_open_message_and_respects_the_byte_budget() {
        let mut prepared = Prepared::default();
        let large = BYTES / 3;
        prepared.finished("open".into(), 0, Some(preview(large)), Some("open"));
        prepared.finished("near".into(), 0, Some(preview(large)), Some("open"));
        prepared.finished("near".into(), 1, Some(preview(large)), Some("open"));
        prepared.finished("near".into(), 2, Some(preview(large)), Some("open"));
        assert!(prepared.bytes() <= BYTES);
        let keys = prepared.keys();
        assert!(keys.contains(&("open".into(), 0)), "{keys:?}");
        assert!(!keys.contains(&("near".into(), 0)), "{keys:?}");
        assert!(prepared.get("near", 2).is_some());
    }

    #[test]
    fn replacing_an_entry_does_not_count_it_twice() {
        let mut prepared = Prepared::default();
        prepared.finished("m".into(), 0, Some(preview(100)), None);
        prepared.finished("m".into(), 0, Some(preview(40)), None);
        assert_eq!(prepared.keys().len(), 1);
        assert_eq!(prepared.bytes(), 40);
    }
}
