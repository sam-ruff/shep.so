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
    items: HashSet<u64>,
    updated: Instant,
}

impl Toast {
    pub fn label(&self) -> String {
        let count = self.items.len();
        let noun = if count == 1 { "message" } else { "messages" };
        if self.folder.eq_ignore_ascii_case("Archive") {
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
    #[cfg(feature = "test-support")]
    pub fn count(&self) -> usize {
        self.items.len()
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
            items: HashSet::new(),
            updated: now,
        });
        let standard_folder =
            folder.eq_ignore_ascii_case("Archive") || folder.eq_ignore_ascii_case("Trash");
        let same_folder = if standard_folder {
            toast.folder.eq_ignore_ascii_case(folder)
        } else {
            toast.folder == folder
        };
        if !same_folder || (!standard_folder && toast.account != account) {
            *toast = Toast {
                account: account.into(),
                folder: folder.into(),
                items: HashSet::new(),
                updated: now,
            };
        }
        toast.items.insert(token);
        toast.updated = now;
        token
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
