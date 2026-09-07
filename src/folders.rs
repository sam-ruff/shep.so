//! Cached mailbox hierarchy. A NIL IMAP delimiter is flat, even for names that
//! contain slashes or dots. Build trees on the storage worker, not in UI handlers.
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NameEncoding {
    #[default]
    Utf8,
    ImapUtf7,
}
impl NameEncoding {
    pub fn display<'a>(self, name: &'a str) -> Cow<'a, str> {
        if self == Self::ImapUtf7
            && name.contains('&')
            && let Some(decoded) = decode_utf7(name)
        {
            return Cow::Owned(decoded);
        }
        Cow::Borrowed(name)
    }
    fn separator(self, path: &str, delimiter: char) -> Option<usize> {
        let mut shifted = false;
        let mut last = None;
        for (index, c) in path.char_indices() {
            if self == Self::ImapUtf7 {
                if shifted {
                    if c == '-' {
                        shifted = false;
                    }
                    continue;
                }
                if c == '&' {
                    shifted = true;
                    continue;
                }
            }
            if c == delimiter {
                last = Some(index);
            }
        }
        last
    }
}

fn decode_utf7(mut source: &str) -> Option<String> {
    use base64::Engine as _;
    let mut output = String::with_capacity(source.len());
    while let Some(start) = source.find('&') {
        output.push_str(&source[..start]);
        source = &source[start + 1..];
        let end = source.find('-')?;
        if end == 0 {
            output.push('&');
        } else {
            let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(source[..end].replace(',', "/"))
                .ok()?;
            if bytes.len() % 2 != 0 {
                return None;
            }
            let units: Vec<_> = bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            output.push_str(&String::from_utf16(&units).ok()?);
        }
        source = &source[end + 1..];
    }
    output.push_str(source);
    Some(output)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub name: String,
    pub delimiter: Option<char>,
    pub selectable: bool,
    #[serde(default)]
    pub encoding: NameEncoding,
}
impl Mailbox {
    pub fn flat(name: String) -> Self {
        Self {
            name,
            delimiter: None,
            selectable: true,
            encoding: NameEncoding::Utf8,
        }
    }
    pub(crate) fn path(&self) -> &str {
        // Some servers advertise a nonselectable container with a trailing
        // delimiter. Keep its exact wire name while grouping its descendants.
        if !self.selectable
            && let Some(delimiter) = self.delimiter
        {
            let mut path = self.name.as_str();
            while let Some(index) = self.encoding.separator(path, delimiter) {
                if index + delimiter.len_utf8() != path.len() {
                    break;
                }
                path = &path[..index];
            }
            if !path.is_empty() {
                return path;
            }
        }
        &self.name
    }
}

#[derive(Debug, Clone)]
pub struct Node {
    pub path: String,
    pub mailbox: Mailbox,
    pub listed: bool,
    pub label: String,
    pub display_path: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}
#[derive(Debug, Clone, Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
    index: HashMap<String, usize>,
}
impl Tree {
    pub fn new(catalog: &[Mailbox]) -> Self {
        let mut tree = Self::default();
        // Preserve LIST/cache order. Ancestors are inserted before descendants,
        // including servers that omit a nonselectable intermediate container.
        for mailbox in catalog {
            if mailbox.name.is_empty() {
                continue;
            }
            let path = mailbox.path();
            let mut lineage = vec![path];
            if let Some(delimiter) = mailbox.delimiter {
                let mut parent = path;
                while let Some(index) = mailbox.encoding.separator(parent, delimiter) {
                    let prefix = &parent[..index];
                    if prefix.is_empty() {
                        break;
                    }
                    lineage.push(prefix);
                    parent = prefix;
                }
            }
            let mut parent = None;
            for path in lineage.into_iter().rev() {
                let id = if let Some(&id) = tree.index.get(path) {
                    id
                } else {
                    let id = tree.nodes.len();
                    let label = parent
                        .map(|p: usize| {
                            let ancestor = &tree.nodes[p].path;
                            &path[ancestor.len() + mailbox.delimiter.unwrap().len_utf8()..]
                        })
                        .filter(|s| !s.is_empty())
                        .unwrap_or(path)
                        .to_owned();
                    let label = mailbox.encoding.display(&label).into_owned();
                    tree.nodes.push(Node {
                        path: path.to_owned(),
                        mailbox: Mailbox {
                            name: path.to_owned(),
                            delimiter: mailbox.delimiter,
                            selectable: false,
                            encoding: mailbox.encoding,
                        },
                        listed: false,
                        label,
                        display_path: mailbox.encoding.display(path).into_owned(),
                        parent,
                        children: vec![],
                    });
                    tree.index.insert(path.to_owned(), id);
                    if let Some(parent) = parent {
                        tree.nodes[parent].children.push(id);
                    } else {
                        tree.roots.push(id);
                    }
                    id
                };
                parent = Some(id);
            }
            if let Some(id) = parent {
                let node = &mut tree.nodes[id];
                // Prefer an actual selectable mailbox over an equivalent
                // trailing-delimiter container advertised in the same LIST.
                if !node.listed || mailbox.selectable || !node.mailbox.selectable {
                    node.mailbox = mailbox.clone();
                    node.display_path = mailbox.encoding.display(&node.path).into_owned();
                }
                node.listed = true;
            }
        }
        tree
    }
    pub fn node(&self, path: &str) -> Option<&Node> {
        self.index.get(path).map(|&id| &self.nodes[id])
    }
    pub fn visible<'a>(
        &'a self,
        expanded: Option<&'a HashSet<String>>,
    ) -> impl Iterator<Item = (usize, &'a Node)> {
        let mut stack: Vec<_> = self.roots.iter().rev().map(|&id| (0usize, id)).collect();
        std::iter::from_fn(move || {
            let (depth, id) = stack.pop()?;
            let node = &self.nodes[id];
            if expanded.is_some_and(|set| set.contains(&node.path)) {
                stack.extend(node.children.iter().rev().map(|&child| (depth + 1, child)));
            }
            Some((depth, node))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn folder(name: &str, delimiter: Option<char>, selectable: bool) -> Mailbox {
        Mailbox {
            name: name.into(),
            delimiter,
            selectable,
            encoding: NameEncoding::Utf8,
        }
    }
    #[test]
    fn delimiter_nil_dots_unicode_and_missing_parents_keep_exact_names() {
        let tree = Tree::new(&[
            folder("INBOX", Some('/'), true),
            folder("Projects/Design/日本語", Some('/'), true),
            folder("Projects", Some('/'), true),
            folder("Shared.Team.Plans", Some('.'), true),
            folder("Flat/Name.With.Dots", None, true),
        ]);
        let roots: Vec<_> = tree.visible(None).map(|(_, n)| n.path.as_str()).collect();
        assert_eq!(
            roots,
            ["INBOX", "Projects", "Shared", "Flat/Name.With.Dots"]
        );
        assert!(tree.node("Projects").unwrap().mailbox.selectable);
        assert!(!tree.node("Projects/Design").unwrap().mailbox.selectable);
        assert!(!tree.node("Shared.Team").unwrap().listed);
        let expanded = ["Projects", "Projects/Design", "Shared", "Shared.Team"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(
            tree.visible(Some(&expanded))
                .map(|(d, n)| (d, n.label.as_str()))
                .collect::<Vec<_>>(),
            [
                (0, "INBOX"),
                (0, "Projects"),
                (1, "Design"),
                (2, "日本語"),
                (0, "Shared"),
                (1, "Team"),
                (2, "Plans"),
                (0, "Flat/Name.With.Dots")
            ]
        );
        assert_eq!(
            tree.node("Shared.Team.Plans").unwrap().mailbox.name,
            "Shared.Team.Plans"
        );
    }
    #[test]
    fn explicit_nonselectable_parents_and_trailing_delimiters_are_preserved() {
        let tree = Tree::new(&[
            folder("Teams/", Some('/'), false),
            folder("Teams/Remote", Some('/'), true),
            folder("Empty", Some('/'), false),
        ]);
        let teams = tree.node("Teams").unwrap();
        assert_eq!(teams.mailbox.name, "Teams/");
        assert!(teams.listed && !teams.mailbox.selectable);
        assert_eq!(teams.children.len(), 1);
        assert_eq!(tree.nodes[teams.children[0]].label, "Remote");
        assert!(tree.node("Empty").unwrap().children.is_empty());
        assert_eq!(tree.roots.len(), 2);
    }
    #[test]
    fn collapsing_a_parent_preserves_hidden_descendants_expansion() {
        let tree = Tree::new(&[folder("A/B/C", Some('/'), true)]);
        let expanded = ["A/B".to_owned()].into_iter().collect();
        assert_eq!(tree.visible(Some(&expanded)).count(), 1);
        let expanded = ["A", "A/B"].into_iter().map(str::to_owned).collect();
        assert_eq!(tree.visible(Some(&expanded)).count(), 3);
    }
    #[test]
    fn utf7_labels_are_decoded_without_changing_wire_names_or_utf8_names() {
        let mailbox = Mailbox {
            name: "~fixture/mail/&U,BTFw-/&ZeVnLIqe-".into(),
            delimiter: Some('/'),
            selectable: true,
            encoding: NameEncoding::ImapUtf7,
        };
        let tree = Tree::new(std::slice::from_ref(&mailbox));
        assert_eq!(tree.node(&mailbox.name).unwrap().label, "日本語");
        assert_eq!(tree.node("~fixture/mail/&U,BTFw-").unwrap().label, "台北");
        assert_eq!(tree.node(&mailbox.name).unwrap().mailbox.name, mailbox.name);
        assert_eq!(
            NameEncoding::ImapUtf7.display("Work &- travel"),
            "Work & travel"
        );
        assert_eq!(NameEncoding::Utf8.display("&ZeVnLIqe-"), "&ZeVnLIqe-");
        for invalid in ["Invalid &missing", "&a-", "&2AA-"] {
            assert_eq!(NameEncoding::ImapUtf7.display(invalid), invalid);
        }
        let tree = Tree::new(&[Mailbox {
            name: "Root-&ZeVnLIqe-".into(),
            delimiter: Some('-'),
            ..mailbox
        }]);
        assert_eq!(tree.node("Root-&ZeVnLIqe-").unwrap().label, "日本語");
        assert_eq!(
            tree.nodes.len(),
            2,
            "the UTF-7 terminator is not a hierarchy separator"
        );
    }
}
