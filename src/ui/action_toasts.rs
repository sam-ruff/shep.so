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
    destination: Destination,
}

/// The physical folder the counted moves reached, which can differ from the
/// logical name the action asked for, such as `Junk Mail` for `Junk`.
#[derive(Default)]
enum Destination {
    #[default]
    Requested,
    Acknowledged {
        account: String,
        folder: String,
    },
    /// Receipts named different folders, so only the requested name is true.
    Mixed,
}

impl Toast {
    fn new(account: &str, folder: &str, now: Instant) -> Self {
        Self {
            account: account.into(),
            folder: folder.into(),
            items: HashMap::new(),
            updated: now,
            restored: false,
            destination: Destination::Requested,
        }
    }
    /// The account and wire folder the label names.
    fn named(&self) -> (&str, &str) {
        match &self.destination {
            Destination::Acknowledged { account, folder } => (account, folder),
            Destination::Requested | Destination::Mixed => (&self.account, &self.folder),
        }
    }
    #[cfg(test)]
    pub fn label(&self) -> String {
        self.label_with_folder(self.named().1)
    }
    pub fn display_label(&self, workspace: &crate::store::Workspace) -> String {
        let (account, folder) = self.named();
        let display = workspace.folder_label((!account.is_empty()).then_some(account), folder);
        self.label_with_folder(&display)
    }
    fn label_with_folder(&self, display_folder: &str) -> String {
        let count: usize = self.items.values().sum();
        let noun = if count == 1 { "message" } else { "messages" };
        if self.restored {
            format!("Restored {count} {noun}")
        } else if self.folder.eq_ignore_ascii_case("Archive") {
            format!("Archived {count} {noun}")
        } else if self.folder.eq_ignore_ascii_case("Trash") {
            format!("Deleted {count} {noun}")
        } else {
            let folder = if display_folder.eq_ignore_ascii_case("INBOX") {
                "Inbox"
            } else {
                display_folder
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
        let toast = self
            .current
            .get_or_insert_with(|| Toast::new(account, folder, now));
        let standard_folder =
            folder.eq_ignore_ascii_case("Archive") || folder.eq_ignore_ascii_case("Trash");
        let same_folder = if standard_folder {
            toast.folder.eq_ignore_ascii_case(folder)
        } else {
            toast.folder == folder || toast.named() == (account, folder)
        };
        if toast.restored || !same_folder || (!standard_folder && toast.account != account) {
            *toast = Toast::new(account, folder, now);
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
            items: tokens.into_iter().collect(),
            restored: true,
            ..Toast::new("", "", now)
        });
    }
    /// Records the folder a counted move's receipt reports. It neither refreshes
    /// the deadline nor recreates a dismissed or expired toast.
    pub fn acknowledged(&mut self, token: u64, account: &str, folder: &str) {
        let Some(toast) = self
            .current
            .as_mut()
            .filter(|toast| !toast.restored && toast.items.contains_key(&token))
        else {
            return;
        };
        toast.destination = match &toast.destination {
            Destination::Requested => Destination::Acknowledged {
                account: account.into(),
                folder: folder.into(),
            },
            Destination::Acknowledged {
                account: known_account,
                folder: known_folder,
            } if known_account == account && known_folder == folder => return,
            Destination::Acknowledged { .. } | Destination::Mixed => Destination::Mixed,
        };
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

    #[test]
    fn receipts_name_the_acknowledged_folder_only_when_they_agree() {
        let label = |toasts: &ActionToasts| toasts.current.as_ref().map(Toast::label);
        let mut toasts = ActionToasts::default();
        let now = Instant::now();
        let first = toasts.add("work", "Junk", now);
        let second = toasts.add("work", "Junk", now);
        toasts.acknowledged(99, "work", "Elsewhere");
        assert_eq!(label(&toasts).unwrap(), "Moved 2 messages to Junk");
        toasts.acknowledged(first, "work", "Junk Mail");
        toasts.acknowledged(second, "work", "Junk Mail");
        assert_eq!(label(&toasts).unwrap(), "Moved 2 messages to Junk Mail");
        toasts.acknowledged(second, "work", "Spam");
        assert_eq!(
            label(&toasts).unwrap(),
            "Moved 2 messages to Junk",
            "conflicting receipts fall back to the requested name"
        );

        let group = toasts.add_group("", "Junk", 3, now);
        toasts.acknowledged(group, "work", "Junk Mail");
        toasts.acknowledged(group, "personal", "Spam");
        assert_eq!(label(&toasts).unwrap(), "Moved 3 messages to Junk");

        let archived = toasts.add("work", "Archive", now);
        toasts.acknowledged(archived, "work", "Archives");
        assert_eq!(label(&toasts).unwrap(), "Archived 1 message");

        let moved = toasts.add("work", "Junk", now);
        toasts.restored(vec![moved], now);
        toasts.acknowledged(moved, "work", "Junk Mail");
        assert_eq!(label(&toasts).unwrap(), "Restored 1 message");

        let late = toasts.add("work", "Junk", now);
        toasts.expire(now + LIFETIME);
        toasts.acknowledged(late, "work", "Junk Mail");
        assert!(
            toasts.current.is_none(),
            "a receipt never recreates a toast"
        );
    }
}
