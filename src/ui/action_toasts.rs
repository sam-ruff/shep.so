use super::*;

const LIFETIME: std::time::Duration = std::time::Duration::from_secs(6);

#[derive(Default)]
pub(super) struct ActionToasts {
    sequence: u64,
    pub current: Option<Toast>,
}

pub(super) struct Toast {
    account: String,
    folder: String,
    items: HashMap<u64, usize>,
    updated: Instant,
    restored: bool,
}

impl Toast {
    pub fn label(&self) -> String {
        let count: usize = self.items.values().sum();
        let noun = if count == 1 { "message" } else { "messages" };
        if self.restored {
            format!("Restored {count} {noun}")
        } else if self.folder.eq_ignore_ascii_case("Archive") {
            format!("Archived {count} {noun}")
        } else if self.folder.eq_ignore_ascii_case("Trash") {
            format!("Deleted {count} {noun}")
        } else {
            let folder = if self.folder.eq_ignore_ascii_case("INBOX") {
                "Inbox"
            } else {
                &self.folder
            };
            format!("Moved {count} {noun} to {folder}")
        }
    }
    pub fn undo_tokens(&self) -> Vec<u64> {
        if self.restored {
            return vec![];
        }
        let mut tokens: Vec<_> = self.items.keys().copied().collect();
        tokens.sort_unstable();
        tokens
    }
    pub fn contains(&self, token: u64) -> bool {
        self.items.contains_key(&token)
    }
    #[cfg(feature = "test-support")]
    pub fn count(&self) -> usize {
        self.items.values().sum()
    }
}

impl ActionToasts {
    pub fn add(&mut self, account: &str, folder: &str, now: Instant) -> u64 {
        self.expire(now);
        self.sequence += 1;
        let token = self.sequence;
        let toast = self.current.get_or_insert_with(|| Toast {
            account: account.into(),
            folder: folder.into(),
            items: HashMap::new(),
            updated: now,
            restored: false,
        });
        let standard_folder =
            folder.eq_ignore_ascii_case("Archive") || folder.eq_ignore_ascii_case("Trash");
        let same_folder = if standard_folder {
            toast.folder.eq_ignore_ascii_case(folder)
        } else {
            toast.folder == folder
        };
        if toast.restored || !same_folder || (!standard_folder && toast.account != account) {
            *toast = Toast {
                account: account.into(),
                folder: folder.into(),
                items: HashMap::new(),
                updated: now,
                restored: false,
            };
        }
        toast.items.insert(token, 1);
        toast.updated = now;
        token
    }
    pub fn restored(&mut self, tokens: Vec<u64>, now: Instant) {
        let tokens: Vec<_> = tokens
            .into_iter()
            .map(|token| (token, self.weight(token)))
            .collect();
        self.restored_counts(tokens, now);
    }
    pub fn weight(&self, token: u64) -> usize {
        self.current
            .as_ref()
            .and_then(|t| t.items.get(&token))
            .copied()
            .unwrap_or(1)
    }
    pub fn add_group(&mut self, account: &str, folder: &str, count: usize, now: Instant) -> u64 {
        let token = self.add(account, folder, now);
        self.set_count(token, count);
        token
    }
    pub fn set_count(&mut self, token: u64, count: usize) {
        if count == 0 {
            self.failed(token);
        } else if let Some(value) = self.current.as_mut().and_then(|t| t.items.get_mut(&token)) {
            *value = count;
        }
    }
    pub fn restored_counts(&mut self, tokens: Vec<(u64, usize)>, now: Instant) {
        self.current = (!tokens.is_empty()).then(|| Toast {
            account: String::new(),
            folder: String::new(),
            items: tokens.into_iter().collect(),
            updated: now,
            restored: true,
        });
    }
    pub fn failed(&mut self, token: u64) {
        if let Some(toast) = &mut self.current {
            toast.items.remove(&token);
            if toast.items.is_empty() {
                self.current = None;
            }
        }
    }
    pub fn expire(&mut self, now: Instant) {
        if self
            .current
            .as_ref()
            .is_some_and(|t| now.saturating_duration_since(t.updated) >= LIFETIME)
        {
            self.current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_refresh_deadline_and_keep_action_destination_and_account_scoped() {
        let mut toasts = ActionToasts::default();
        let now = Instant::now();
        toasts.add("work", "Archive", now);
        assert_eq!(
            toasts.current.as_ref().unwrap().label(),
            "Archived 1 message"
        );
        let second = toasts.add("personal", "Archive", now + LIFETIME / 2);
        toasts.expire(now + LIFETIME);
        assert_eq!(
            toasts.current.as_ref().unwrap().label(),
            "Archived 2 messages"
        );
        toasts.failed(second);
        assert_eq!(
            toasts.current.as_ref().unwrap().label(),
            "Archived 1 message"
        );
        let trash = toasts.add("work", "Trash", now + LIFETIME);
        assert_eq!(
            toasts.current.as_ref().unwrap().label(),
            "Deleted 1 message"
        );
        toasts.failed(second);
        assert_eq!(
            toasts.current.as_ref().unwrap().label(),
            "Deleted 1 message"
        );
        toasts.failed(trash);
        assert!(toasts.current.is_none());
        toasts.add("work", "Plans", now);
        toasts.add("personal", "Plans", now);
        assert_eq!(
            toasts.current.as_ref().unwrap().label(),
            "Moved 1 message to Plans"
        );
        toasts.add("personal", "INBOX", now);
        assert_eq!(
            toasts.current.as_ref().unwrap().label(),
            "Moved 1 message to Inbox"
        );
        toasts.expire(now + LIFETIME);
        assert!(toasts.current.is_none());
        toasts.add("personal", "INBOX", now + LIFETIME);
        assert_eq!(
            toasts.current.as_ref().unwrap().label(),
            "Moved 1 message to Inbox"
        );
    }
}
