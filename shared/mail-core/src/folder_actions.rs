//! Reviewed mailbox changes. Paths remain exact provider identities; display
//! labels and IMAP delimiters must never be substituted into wire commands.
//! The store-driven runner lives with each client; this module is the plan,
//! review and provider connection contract.
use crate::folders::{Mailbox, Tree};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Move { parent: Option<String> },
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub path: String,
    pub mailbox: Mailbox,
    pub listed: bool,
    pub depth: usize,
    pub destination: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub source: String,
    pub action: Action,
    /// Deepest first, so DELETE never assumes recursive server semantics.
    pub members: Vec<Member>,
    pub parent: Option<Mailbox>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    Rename {
        source: String,
        destination: String,
    },
    Delete {
        source: String,
    },
    /// An inferred/NonExistent container has no server mailbox to DELETE.
    Forget {
        source: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Applied,
    Rejected(String),
    Uncertain(String),
}

#[async_trait::async_trait]
pub trait Connection: Send {
    async fn catalog(&mut self) -> anyhow::Result<Vec<Mailbox>>;
    async fn apply(&mut self, step: &Step) -> Outcome;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub account: String,
    pub connection: String,
    pub imap: bool,
    pub plan: Plan,
    pub cached_messages: usize,
    pub affected_history: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Running,
    Acknowledged,
    Done,
    Rejected,
    Uncertain,
    Accepted,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub position: usize,
    pub step: Step,
    pub status: Status,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Job {
    /// Scalar count of remaining mail in the original reviewed UI scope.
    pub query_counts: Option<(usize, usize)>,
    pub revision: u64,
    /// Decoded on the storage worker; retained after the source leaves LIST.
    pub label: String,
    pub destination_label: Option<String>,
    pub id: String,
    pub review: Review,
    pub steps: Vec<Progress>,
    pub closed: bool,
}

impl Job {
    /// Recheck the remaining scope before each destructive command. A server may
    /// remove an empty NoSelect ancestor after its final child was deleted.
    pub fn preflight(&self, catalog: &[Mailbox]) -> anyhow::Result<HashSet<String>> {
        if matches!(self.review.plan.action, Action::Move { .. }) {
            self.review.plan.revalidate(catalog)?;
            return Ok(HashSet::new());
        }
        let deleted: HashSet<_> = self
            .steps
            .iter()
            .filter(|step| step.status == Status::Done)
            .filter_map(|step| match &step.step {
                Step::Delete { source } | Step::Forget { source } => Some(source.as_str()),
                _ => None,
            })
            .collect();
        let remaining: HashMap<_, _> = self
            .review
            .plan
            .members
            .iter()
            .filter(|member| !deleted.contains(member.mailbox.name.as_str()))
            .map(|member| (member.path.as_str(), member))
            .collect();
        let tree = Tree::new(catalog);
        let mut absent = HashSet::new();
        for member in remaining.values() {
            if let Some(node) = tree.node(&member.path) {
                anyhow::ensure!(
                    node.mailbox == member.mailbox && node.listed == member.listed,
                    "A remaining folder changed. Review the folder list before continuing."
                );
            } else {
                anyhow::ensure!(
                    !member.mailbox.selectable,
                    "A remaining folder is no longer present. Review the folder list before continuing."
                );
                absent.insert(member.mailbox.name.clone());
            }
        }
        if let Some(root) = tree.node(&self.review.plan.source) {
            let mut nodes = vec![root];
            while let Some(node) = nodes.pop() {
                anyhow::ensure!(
                    remaining.contains_key(node.path.as_str()),
                    "A folder appeared inside the reviewed subtree. Review the new scope before continuing."
                );
                nodes.extend(node.children.iter().map(|&id| &tree.nodes[id]));
            }
        }
        Ok(absent)
    }

    pub fn confirm_renamed_catalog(&self, catalog: &[Mailbox]) -> anyhow::Result<()> {
        let tree = Tree::new(catalog);
        for member in &self.review.plan.members {
            if member.listed && !member.mailbox.non_existent {
                let destination =
                    Plan::wire_destination(member).context("This change has no destination.")?;
                anyhow::ensure!(
                    tree.node(member.destination.as_deref().unwrap())
                        .is_some_and(|node| node.mailbox.selectable == member.mailbox.selectable
                            && node.mailbox.name == destination),
                    "The server acknowledged the move but returned different folder names. The original cache is retained; refresh and review the destination before finishing this change."
                );
            }
        }
        Ok(())
    }
}

impl Plan {
    pub fn new(tree: &Tree, source: &str, action: Action) -> anyhow::Result<Self> {
        let root = tree
            .node(source)
            .context("This folder changed. Refresh the folder list.")?;
        anyhow::ensure!(
            !root.path.eq_ignore_ascii_case("INBOX"),
            "Inbox cannot be moved or deleted."
        );
        valid_name(&root.mailbox.name)?;
        let mut members = Vec::new();
        let mut stack = vec![(root, 0)];
        while let Some((node, depth)) = stack.pop() {
            valid_name(&node.mailbox.name)?;
            anyhow::ensure!(
                node.mailbox.delimiter == root.mailbox.delimiter
                    && node.mailbox.encoding == root.mailbox.encoding,
                "The server reported mixed namespaces in this folder. Refresh before changing it."
            );
            members.push(Member {
                path: node.path.clone(),
                mailbox: node.mailbox.clone(),
                listed: node.listed,
                depth,
                destination: None,
            });
            stack.extend(
                node.children
                    .iter()
                    .map(|&child| (&tree.nodes[child], depth + 1)),
            );
        }
        members.sort_by(|a, b| b.depth.cmp(&a.depth).then_with(|| a.path.cmp(&b.path)));
        let parent = if let Action::Move { parent } = &action {
            let delimiter = root
                .mailbox
                .delimiter
                .context("This server's flat namespace does not support nesting folders.")?;
            let parent = parent
                .as_ref()
                .map(|name| {
                    tree.node(name)
                        .context("The destination folder changed. Refresh the folder list.")
                })
                .transpose()?;
            if let Some(parent) = parent {
                anyhow::ensure!(
                    !members.iter().any(|member| member.path == parent.path),
                    "A folder cannot be moved inside itself or its children."
                );
                anyhow::ensure!(
                    !parent.mailbox.no_inferiors,
                    "This folder cannot contain other folders."
                );
                anyhow::ensure!(
                    parent.mailbox.delimiter == Some(delimiter)
                        && parent.mailbox.encoding == root.mailbox.encoding,
                    "Choose a folder in the same server namespace."
                );
            }
            let leaf = root.parent.map_or(root.path.as_str(), |id| {
                &root.path[tree.nodes[id].path.len() + delimiter.len_utf8()..]
            });
            let destination = parent.map_or_else(
                || leaf.to_owned(),
                |p| format!("{}{delimiter}{leaf}", p.path),
            );
            anyhow::ensure!(
                destination != root.path,
                "This folder is already in that location."
            );
            let source_paths: HashSet<_> =
                members.iter().map(|member| member.path.clone()).collect();
            for member in &mut members {
                let path = format!("{}{}", destination, &member.path[root.path.len()..]);
                valid_name(&path)?;
                anyhow::ensure!(
                    tree.node(&path).is_none() || source_paths.contains(&path),
                    "A folder already exists at the destination."
                );
                member.destination = Some(path);
            }
            parent.map(|p| p.mailbox.clone())
        } else {
            None
        };
        Ok(Self {
            source: root.path.clone(),
            action,
            members,
            parent,
        })
    }

    pub fn steps(&self) -> Vec<Step> {
        match self.action {
            Action::Move { .. } => {
                let root = self
                    .members
                    .iter()
                    .find(|member| member.path == self.source)
                    .expect("A plan contains its root");
                vec![Step::Rename {
                    source: root.mailbox.name.clone(),
                    destination: Self::wire_destination(root).expect("A move has a destination"),
                }]
            }
            Action::Delete => self
                .members
                .iter()
                .map(|member| {
                    if member.listed && !member.mailbox.non_existent {
                        Step::Delete {
                            source: member.mailbox.name.clone(),
                        }
                    } else {
                        Step::Forget {
                            source: member.mailbox.name.clone(),
                        }
                    }
                })
                .collect(),
        }
    }

    pub fn wire_destination(member: &Member) -> Option<String> {
        member.destination.as_ref().map(|destination| {
            format!("{destination}{}", &member.mailbox.name[member.path.len()..])
        })
    }

    /// The review freezes the affected subtree, not unrelated LIST ordering.
    /// New descendants or changed target capabilities require another review.
    pub fn revalidate(&self, catalog: &[Mailbox]) -> anyhow::Result<()> {
        let current = Self::new(&Tree::new(catalog), &self.source, self.action.clone())?;
        anyhow::ensure!(
            &current == self,
            "The folders changed since this review. Review the new folder list before continuing."
        );
        Ok(())
    }

    /// Project or commit the acknowledged part of a change. Other catalog
    /// entries retain their exact LIST metadata and order.
    pub fn project(&self, catalog: &[Mailbox], completed: &[Step]) -> Vec<Mailbox> {
        let renamed = completed
            .iter()
            .any(|step| matches!(step, Step::Rename { .. }));
        let deleted: HashSet<_> = completed
            .iter()
            .filter_map(|step| match step {
                Step::Delete { source } | Step::Forget { source } => Some(source.as_str()),
                _ => None,
            })
            .collect();
        let mapping: HashMap<_, _> = self
            .members
            .iter()
            .map(|member| (member.mailbox.name.as_str(), member))
            .collect();
        catalog
            .iter()
            .filter_map(|mailbox| {
                if deleted.contains(mailbox.name.as_str()) {
                    return None;
                }
                let mut mailbox = mailbox.clone();
                if renamed
                    && let Some(member) = mapping.get(mailbox.name.as_str())
                    && let Some(name) = Self::wire_destination(member)
                {
                    mailbox.name = name;
                }
                Some(mailbox)
            })
            .collect()
    }
}

pub fn valid_name(name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !name.is_empty() && !name.contains(['\r', '\n', '\0']),
        "Choose a valid folder name."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::NameEncoding;
    fn folder(name: &str) -> Mailbox {
        Mailbox {
            delimiter: Some('/'),
            encoding: NameEncoding::ImapUtf7,
            ..Mailbox::flat(name.into())
        }
    }
    fn catalog() -> Vec<Mailbox> {
        [
            "INBOX",
            "Projects",
            "Projects/Design",
            "Projects/Design/&ZeVnLIqe-",
            "ProjectsElsewhere",
            "Archive",
            "Archive/Old",
        ]
        .into_iter()
        .map(folder)
        .collect()
    }
    #[test]
    fn move_reviews_whole_subtree_and_preserves_wire_names() {
        let catalog = catalog();
        let plan = Plan::new(
            &Tree::new(&catalog),
            "Projects",
            Action::Move {
                parent: Some("Archive".into()),
            },
        )
        .unwrap();
        assert_eq!(plan.members.len(), 3);
        assert_eq!(
            plan.steps(),
            [Step::Rename {
                source: "Projects".into(),
                destination: "Archive/Projects".into()
            }]
        );
        let after = Tree::new(&plan.project(&catalog, &plan.steps()));
        assert_eq!(
            after
                .node("Archive/Projects/Design/&ZeVnLIqe-")
                .unwrap()
                .label,
            "日本語"
        );
        assert!(after.node("Projects").is_none());
        assert!(after.node("ProjectsElsewhere").is_some());
        assert!(after.node("Archive/Old").is_some());
    }
    #[test]
    fn destination_validation_rejects_cycles_collisions_and_namespace_changes() {
        let mut catalog = catalog();
        catalog.push(folder("Archive/Projects"));
        let tree = Tree::new(&catalog);
        for parent in ["Projects", "Projects/Design", "Archive", "missing"] {
            assert!(
                Plan::new(
                    &tree,
                    "Projects",
                    Action::Move {
                        parent: Some(parent.into())
                    }
                )
                .is_err(),
                "{parent}"
            );
        }
        for action in [
            Action::Delete,
            Action::Move {
                parent: Some("Archive".into()),
            },
        ] {
            assert!(Plan::new(&tree, "INBOX", action).is_err());
        }
        let mut target = folder("Flat");
        target.no_inferiors = true;
        catalog.push(target);
        assert!(
            Plan::new(
                &Tree::new(&catalog),
                "Projects",
                Action::Move {
                    parent: Some("Flat".into())
                }
            )
            .is_err()
        );
        catalog.push(Mailbox::flat("Literal/Flat".into()));
        assert!(
            Plan::new(
                &Tree::new(&catalog),
                "Literal/Flat",
                Action::Move {
                    parent: Some("Archive".into())
                }
            )
            .is_err()
        );
    }
    #[test]
    fn changed_review_requires_confirmation_again_but_list_order_does_not() {
        let mut catalog = catalog();
        let plan = Plan::new(&Tree::new(&catalog), "Projects", Action::Delete).unwrap();
        catalog.reverse();
        assert!(plan.revalidate(&catalog).is_ok());
        catalog.push(folder("Unrelated"));
        assert!(plan.revalidate(&catalog).is_ok());
        catalog.push(folder("Projects/New mail"));
        assert!(plan.revalidate(&catalog).is_err());
    }
    #[test]
    fn delete_is_deepest_first_and_partial_projection_preserves_remaining_folders() {
        let mut catalog = catalog();
        catalog.push(Mailbox {
            selectable: false,
            non_existent: true,
            ..folder("Projects/Ghost")
        });
        let plan = Plan::new(&Tree::new(&catalog), "Projects", Action::Delete).unwrap();
        let steps = plan.steps();
        assert_eq!(steps.len(), 4);
        assert_eq!(
            steps[0],
            Step::Delete {
                source: "Projects/Design/&ZeVnLIqe-".into()
            }
        );
        assert_eq!(
            steps[3],
            Step::Delete {
                source: "Projects".into()
            }
        );
        let after = Tree::new(&plan.project(&catalog, &steps[..1]));
        assert!(after.node("Projects/Design").is_some());
        assert!(after.node("Projects/Design/&ZeVnLIqe-").is_none());
    }
    #[test]
    fn root_moves_and_trailing_containers_preserve_server_spelling() {
        let catalog = [
            folder("Teams/Remote/Plans"),
            Mailbox {
                selectable: false,
                ..folder("Teams/Remote/")
            },
        ];
        let plan = Plan::new(
            &Tree::new(&catalog),
            "Teams/Remote",
            Action::Move { parent: None },
        )
        .unwrap();
        assert_eq!(
            plan.steps(),
            [Step::Rename {
                source: "Teams/Remote/".into(),
                destination: "Remote/".into()
            }]
        );
        let after = plan.project(&catalog, &plan.steps());
        assert_eq!(after[0].name, "Remote/Plans");
        assert_eq!(after[1].name, "Remote/");
    }
}
