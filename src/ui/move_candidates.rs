//! Destinations for the Move chooser: the acted account's folders plus, while
//! the user types, matching folders from other IMAP accounts.
use crate::model::{Account, Protocol};
use crate::store::Workspace;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MoveCandidate {
    pub account: String,
    pub folder: String,
    pub label: String,
    pub foreign: bool,
    /// Position of the account in the workspace listing, the final tie-break.
    pub order: usize,
}

/// Where the unbadged folders come from.
pub(super) enum Source<'a> {
    /// The acted message's account, when one is resolved.
    Message(Option<&'a str>),
    /// The captured selection's accounts, when selection mode has a snapshot.
    Selection(Option<Vec<&'a str>>),
}

pub(super) struct Home<'a> {
    /// The account chosen in the destination pick list, if any.
    pub explicit: Option<&'a str>,
    pub source: Source<'a>,
    /// The account whose tree decodes the home labels.
    pub label_account: Option<&'a str>,
}

/// IMAP accounts other than the home ones. Nothing is foreign without a home
/// account, and a POP3 source never reaches another account.
pub(super) fn foreign_accounts<'a>(
    accounts: &'a [Account],
    home: &[&str],
    enabled: bool,
) -> Vec<&'a Account> {
    if !enabled || home.is_empty() {
        return Vec::new();
    }
    let imap = |id: &str| {
        accounts
            .iter()
            .any(|a| a.id == id && a.protocol == Protocol::Imap)
    };
    if !home.iter().all(|id| imap(id)) {
        return Vec::new();
    }
    accounts
        .iter()
        .filter(|a| a.protocol == Protocol::Imap && !home.contains(&a.id.as_str()))
        .collect()
}

fn home_folders<'a>(workspace: &Workspace, home: &Home<'a>) -> (Vec<&'a str>, Vec<String>) {
    let own = |id: &str| {
        workspace
            .account_folders
            .get(id)
            .cloned()
            .unwrap_or_else(|| workspace.folders.clone())
    };
    if let Some(id) = home.explicit {
        return (vec![id], own(id));
    }
    match &home.source {
        Source::Message(account) => (
            account.iter().copied().collect(),
            own(account.unwrap_or("")),
        ),
        Source::Selection(None) => (Vec::new(), Vec::new()),
        Source::Selection(Some(accounts)) => {
            let mut accounts = accounts.iter().copied();
            let Some(first) = accounts.next() else {
                return (Vec::new(), Vec::new());
            };
            let mut folders = workspace
                .account_folders
                .get(first)
                .cloned()
                .unwrap_or_default();
            let mut ids = vec![first];
            for account in accounts {
                folders.retain(|folder| {
                    workspace
                        .account_folders
                        .get(account)
                        .is_some_and(|f| f.contains(folder))
                });
                ids.push(account);
            }
            (ids, folders)
        }
    }
}

fn order(workspace: &Workspace, id: &str) -> usize {
    workspace
        .accounts
        .iter()
        .position(|a| a.id == id)
        .unwrap_or(usize::MAX)
}

/// Home rows first, in catalogue order, then one row per foreign account and
/// folder. Foreign rows appear only while typing and never for an explicit
/// pick-list account.
pub(super) fn gather(
    workspace: &Workspace,
    home: &Home<'_>,
    enabled: bool,
    query_is_empty: bool,
) -> Vec<MoveCandidate> {
    let (home_accounts, folders) = home_folders(workspace, home);
    let home_id = home_accounts.first().copied().unwrap_or("");
    let mut candidates: Vec<_> = folders
        .into_iter()
        .map(|folder| MoveCandidate {
            account: home_id.to_owned(),
            label: workspace
                .folder_label(home.label_account, &folder)
                .into_owned(),
            folder,
            foreign: false,
            order: order(workspace, home_id),
        })
        .collect();
    if query_is_empty || home.explicit.is_some() {
        return candidates;
    }
    for account in foreign_accounts(&workspace.accounts, &home_accounts, enabled) {
        let Some(folders) = workspace.account_folders.get(&account.id) else {
            continue;
        };
        candidates.extend(folders.iter().map(|folder| {
            MoveCandidate {
                account: account.id.clone(),
                folder: folder.clone(),
                label: workspace
                    .folder_label(Some(&account.id), folder)
                    .into_owned(),
                foreign: true,
                order: order(workspace, &account.id),
            }
        }));
    }
    candidates
}

/// Rank labels with the shared Move ranking, which the browser also uses. Ties
/// fall back to the normalised label, the wire name and then the account order,
/// so home-only results keep their exact previous order.
pub(super) fn rank(query: &str, candidates: Vec<MoveCandidate>) -> Vec<MoveCandidate> {
    shep_mail_content::fuzzy::rank_moves(query, candidates, |candidate| {
        shep_mail_content::fuzzy::MoveKey {
            label: &candidate.label,
            folder: &candidate.folder,
            foreign: candidate.foreign,
            order: candidate.order,
        }
    })
}

/// The sidebar's account name: the email unless another account shares it,
/// then the account name.
pub(super) fn account_display<'a>(accounts: &'a [Account], id: &str) -> Option<&'a str> {
    let account = accounts.iter().find(|a| a.id == id)?;
    let duplicated = accounts.iter().filter(|a| a.email == account.email).count() > 1;
    Some(if duplicated && !account.name.trim().is_empty() {
        &account.name
    } else {
        &account.email
    })
}

#[cfg(test)]
pub(super) fn test_account(id: &str, email: &str, protocol: Protocol) -> Account {
    use crate::model::*;
    Account {
        id: id.into(),
        name: format!("{id} name"),
        email: email.into(),
        protocol,
        host: "imap.example".into(),
        port: 993,
        username: email.into(),
        smtp_host: "smtp.example".into(),
        smtp_port: 465,
        incoming_security: ConnectionSecurity::Tls,
        incoming_auth: IncomingAuth::Password,
        smtp_security: None,
        smtp_auth: SmtpAuth::Automatic,
        smtp_username: String::new(),
        smtp_separate_password: false,
        sent_copy: Default::default(),
        sent_folder: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{test_account as account, *};

    fn workspace() -> Workspace {
        let mut workspace = Workspace {
            accounts: vec![
                account("work", "alex@studio.example", Protocol::Imap),
                account("personal", "alex@example.com", Protocol::Imap),
                account("club", "alex@club.example", Protocol::Imap),
                account("pop", "alex@pop.example", Protocol::Pop3),
            ],
            ..Default::default()
        };
        for (id, folders) in [
            (
                "work",
                vec!["INBOX", "Archive", "Archives", "Projects/Archive"],
            ),
            ("personal", vec!["INBOX", "Archive", "Home.Plans"]),
            ("club", vec!["INBOX", "Home.Plans"]),
            ("pop", vec!["INBOX", "Archive"]),
        ] {
            workspace
                .account_folders
                .insert(id.into(), folders.into_iter().map(String::from).collect());
        }
        workspace
    }

    fn message(id: &str) -> Home<'_> {
        Home {
            explicit: None,
            source: Source::Message(Some(id)),
            label_account: Some(id),
        }
    }

    fn ranked(query: &str, home: &Home<'_>, enabled: bool) -> Vec<(String, String, bool)> {
        rank(
            query,
            gather(&workspace(), home, enabled, query.trim().is_empty()),
        )
        .into_iter()
        .map(|c| (c.account, c.folder, c.foreign))
        .collect()
    }

    #[test]
    fn identical_names_rank_the_home_account_first() {
        let rows = ranked("archive", &message("work"), true);
        assert_eq!(
            rows,
            vec![
                ("work".into(), "Archive".into(), false),
                ("work".into(), "Projects/Archive".into(), false),
                ("personal".into(), "Archive".into(), true),
                ("work".into(), "Archives".into(), false),
            ]
        );
    }

    #[test]
    fn foreign_exact_match_outranks_home_abbreviation_and_typo_matches() {
        let rows = ranked("arch", &message("work"), true);
        assert_eq!(
            rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
            ["Archive", "Archives", "Archive", "Projects/Archive"]
        );
        assert_eq!(rows[2].0, "personal");
        let rows = ranked("plans", &message("work"), true);
        assert_eq!(
            rows,
            vec![
                ("personal".into(), "Home.Plans".into(), true),
                ("club".into(), "Home.Plans".into(), true),
            ]
        );
        let rows = ranked("archvie", &message("personal"), true);
        assert_eq!(rows[0], ("personal".into(), "Archive".into(), false));
        assert_eq!(rows[1], ("work".into(), "Archive".into(), true));
    }

    #[test]
    fn empty_query_lists_only_home_folders_in_catalogue_order() {
        let rows = ranked("", &message("work"), true);
        assert!(rows.iter().all(|r| !r.2 && r.0 == "work"));
        assert_eq!(rows.len(), 4);
        assert_eq!(ranked("   ", &message("work"), true).len(), 4);
    }

    #[test]
    fn pop3_accounts_are_skipped_and_the_home_account_is_never_foreign() {
        let rows = ranked("archive", &message("work"), true);
        assert!(rows.iter().all(|r| r.0 != "pop"));
        assert!(rows.iter().all(|r| !(r.0 == "work" && r.2)));
        assert!(
            ranked("archive", &message("pop"), true)
                .iter()
                .all(|r| !r.2)
        );
    }

    #[test]
    fn explicit_account_yields_no_foreign_rows() {
        let home = Home {
            explicit: Some("personal"),
            source: Source::Message(Some("work")),
            label_account: Some("personal"),
        };
        let rows = ranked("archive", &home, true);
        assert_eq!(rows, vec![("personal".into(), "Archive".into(), false)]);
    }

    #[test]
    fn selection_intersects_home_folders_and_adds_foreign_rows_only_when_all_imap() {
        let home = Home {
            explicit: None,
            source: Source::Selection(Some(vec!["work", "personal"])),
            label_account: Some("work"),
        };
        let rows = ranked("", &home, true);
        assert_eq!(
            rows.iter().map(|r| r.1.as_str()).collect::<Vec<_>>(),
            ["Archive", "INBOX"]
        );
        let rows = ranked("plans", &home, true);
        assert_eq!(rows, vec![("club".into(), "Home.Plans".into(), true)]);
        let mixed = Home {
            explicit: None,
            source: Source::Selection(Some(vec!["work", "pop"])),
            label_account: Some("work"),
        };
        assert!(ranked("plans", &mixed, true).is_empty());
        let none = Home {
            explicit: None,
            source: Source::Selection(None),
            label_account: None,
        };
        assert!(ranked("plans", &none, true).is_empty());
    }

    #[test]
    fn disabled_preference_yields_no_foreign_rows() {
        let rows = ranked("plans", &message("work"), false);
        assert!(rows.is_empty());
        assert!(foreign_accounts(&workspace().accounts, &[], true).is_empty());
    }

    #[test]
    fn account_display_uses_email_unless_duplicated() {
        let mut accounts = workspace().accounts;
        assert_eq!(
            account_display(&accounts, "work"),
            Some("alex@studio.example")
        );
        accounts[2].email = "alex@studio.example".into();
        assert_eq!(account_display(&accounts, "work"), Some("work name"));
        accounts[0].name = " ".into();
        assert_eq!(
            account_display(&accounts, "work"),
            Some("alex@studio.example")
        );
        assert_eq!(account_display(&accounts, "missing"), None);
    }
}
