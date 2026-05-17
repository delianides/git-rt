use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::git::{ChangeGroup, FileEntry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowId {
    Directory(String),
    File(String),
    Group(ChangeGroup),
}

#[derive(Debug, Clone)]
pub enum VisibleRow {
    Directory {
        id: RowId,
        depth: usize,
        label: String,
        expanded: bool,
    },
    File {
        id: RowId,
        depth: usize,
        label: String,
        file: FileEntry,
    },
    Header {
        id: RowId,
        label: String,
        count: usize,
        collapsed: bool,
    },
}

impl VisibleRow {
    pub fn id(&self) -> &RowId {
        match self {
            VisibleRow::Directory { id, .. }
            | VisibleRow::File { id, .. }
            | VisibleRow::Header { id, .. } => id,
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            VisibleRow::Directory { depth, .. } | VisibleRow::File { depth, .. } => *depth,
            VisibleRow::Header { .. } => 0,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            VisibleRow::Directory { label, .. }
            | VisibleRow::File { label, .. }
            | VisibleRow::Header { label, .. } => label,
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self, VisibleRow::Directory { .. })
    }

    pub fn is_header(&self) -> bool {
        matches!(self, VisibleRow::Header { .. })
    }

    pub fn directory_expanded(&self) -> Option<bool> {
        match self {
            VisibleRow::Directory { expanded, .. } => Some(*expanded),
            VisibleRow::File { .. } | VisibleRow::Header { .. } => None,
        }
    }

    pub fn header_collapsed(&self) -> Option<bool> {
        match self {
            VisibleRow::Header { collapsed, .. } => Some(*collapsed),
            VisibleRow::Directory { .. } | VisibleRow::File { .. } => None,
        }
    }

    pub fn file(&self) -> Option<&FileEntry> {
        match self {
            VisibleRow::Directory { .. } | VisibleRow::Header { .. } => None,
            VisibleRow::File { file, .. } => Some(file),
        }
    }
}

#[derive(Debug, Default)]
struct DirNode {
    dirs: BTreeMap<String, DirNode>,
    files: Vec<FileEntry>,
}

pub fn build_visible_rows(files: &[FileEntry], expanded: &BTreeSet<String>) -> Vec<VisibleRow> {
    let mut root = DirNode::default();
    for file in files {
        insert_file(&mut root, file.clone());
    }

    let shared_prefix = shared_directory_prefix(files);
    let mut rows = Vec::new();

    if let Some(prefix) = shared_prefix.as_deref() {
        let is_expanded = expanded.contains(prefix);
        rows.push(VisibleRow::Directory {
            id: RowId::Directory(prefix.to_string()),
            depth: 0,
            label: format!("{prefix}/"),
            expanded: is_expanded,
        });

        if is_expanded {
            if let Some(node) = find_node(&root, prefix) {
                flatten_children(node, prefix, 1, expanded, &mut rows);
            }
        }
    } else {
        flatten_children(&root, "", 0, expanded, &mut rows);
    }

    rows
}

fn insert_file(root: &mut DirNode, file: FileEntry) {
    let mut node = root;
    let mut parts = file.path.split('/').peekable();

    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            node.files.push(file);
            return;
        }

        node = node.dirs.entry(part.to_string()).or_default();
    }
}

fn shared_directory_prefix(files: &[FileEntry]) -> Option<String> {
    if files.len() < 2 {
        return None;
    }

    let mut prefixes = files.iter().map(|file| {
        file.path
            .rsplit_once('/')
            .map(|(prefix, _)| prefix.to_string())
            .unwrap_or_default()
    });

    let first = prefixes.next()?;
    let mut shared: Vec<&str> = first.split('/').collect();

    for prefix in prefixes {
        let parts: Vec<&str> = prefix.split('/').collect();
        let mut keep = 0usize;
        while keep < shared.len() && keep < parts.len() && shared[keep] == parts[keep] {
            keep += 1;
        }
        shared.truncate(keep);
        if shared.is_empty() {
            return None;
        }
    }

    if shared.len() == 1 && shared[0].is_empty() {
        None
    } else {
        Some(shared.join("/"))
    }
}

fn find_node<'a>(root: &'a DirNode, path: &str) -> Option<&'a DirNode> {
    let mut node = root;
    if path.is_empty() {
        return Some(node);
    }

    for part in path.split('/') {
        node = node.dirs.get(part)?;
    }

    Some(node)
}

fn flatten_children(
    node: &DirNode,
    current_path: &str,
    depth: usize,
    expanded: &BTreeSet<String>,
    out: &mut Vec<VisibleRow>,
) {
    for (name, child) in &node.dirs {
        let path = if current_path.is_empty() {
            name.clone()
        } else {
            format!("{current_path}/{name}")
        };
        let is_expanded = expanded.contains(&path);
        out.push(VisibleRow::Directory {
            id: RowId::Directory(path.clone()),
            depth,
            label: format!("{name}/"),
            expanded: is_expanded,
        });
        if is_expanded {
            flatten_children(child, &path, depth + 1, expanded, out);
        }
    }

    let mut files: Vec<&FileEntry> = node.files.iter().collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));

    for file in files {
        let label = file
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&file.path)
            .to_string();
        out.push(VisibleRow::File {
            id: RowId::File(file.path.clone()),
            depth,
            label,
            file: file.clone(),
        });
    }
}

/// Group order for the Expanded view.
const EXPANDED_GROUP_ORDER: [ChangeGroup; 3] = [
    ChangeGroup::Changes,
    ChangeGroup::New,
    ChangeGroup::Committed,
];

/// Human-readable label for a status group header.
fn group_label(group: ChangeGroup) -> &'static str {
    match group {
        ChangeGroup::Changes => "Changes",
        ChangeGroup::New => "New files",
        ChangeGroup::Committed => "Committed",
    }
}

/// Build the visible rows for the Expanded view: a collapsible header per
/// non-empty status group, followed by that group's files (flat, sorted by
/// path) unless the group is collapsed.
pub fn build_expanded_rows(
    files: &[FileEntry],
    collapsed: &HashSet<ChangeGroup>,
) -> Vec<VisibleRow> {
    let mut rows = Vec::new();
    for group in EXPANDED_GROUP_ORDER {
        let mut group_files: Vec<&FileEntry> = files.iter().filter(|f| f.group == group).collect();
        if group_files.is_empty() {
            continue;
        }
        group_files.sort_by(|a, b| a.path.cmp(&b.path));

        let is_collapsed = collapsed.contains(&group);
        rows.push(VisibleRow::Header {
            id: RowId::Group(group),
            label: group_label(group).to_string(),
            count: group_files.len(),
            collapsed: is_collapsed,
        });

        if !is_collapsed {
            for file in group_files {
                rows.push(VisibleRow::File {
                    id: RowId::File(file.path.clone()),
                    depth: 1,
                    label: file.path.clone(),
                    file: file.clone(),
                });
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{ChangeGroup, FileEntry, FileStatus};

    fn file(path: &str, ins: usize, del: usize) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: ins,
            deletions: del,
            group: ChangeGroup::Changes,
        }
    }

    fn grouped(path: &str, group: ChangeGroup) -> FileEntry {
        FileEntry {
            path: path.to_string(),
            status: FileStatus::Modified,
            insertions: 1,
            deletions: 0,
            group,
        }
    }

    #[test]
    fn expanded_rows_group_order_and_empty_group_hiding() {
        let files = vec![
            grouped("b.rs", ChangeGroup::Committed),
            grouped("a.rs", ChangeGroup::Changes),
            grouped("c.rs", ChangeGroup::Changes),
        ];
        let rows = build_expanded_rows(&files, &std::collections::HashSet::new());
        // Changes group first (2 files, sorted), then Committed. No New group.
        assert_eq!(rows.len(), 5);
        assert!(rows[0].is_header());
        assert_eq!(rows[0].label(), "Changes");
        assert_eq!(rows[1].label(), "a.rs");
        assert_eq!(rows[2].label(), "c.rs");
        assert!(rows[3].is_header());
        assert_eq!(rows[3].label(), "Committed");
        assert_eq!(rows[4].label(), "b.rs");
    }

    #[test]
    fn expanded_rows_collapsed_group_hides_files() {
        let files = vec![grouped("a.rs", ChangeGroup::Changes)];
        let collapsed = [ChangeGroup::Changes].into_iter().collect();
        let rows = build_expanded_rows(&files, &collapsed);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_header());
        assert_eq!(rows[0].header_collapsed(), Some(true));
    }

    #[test]
    fn header_row_exposes_label_count_and_collapsed() {
        let row = VisibleRow::Header {
            id: RowId::Group(ChangeGroup::Changes),
            label: "Changes".to_string(),
            count: 3,
            collapsed: true,
        };
        assert!(row.is_header());
        assert_eq!(row.label(), "Changes");
        assert_eq!(row.depth(), 0);
        assert!(!row.is_directory());
        assert_eq!(row.directory_expanded(), None);
        assert!(row.file().is_none());
        assert_eq!(row.header_collapsed(), Some(true));
        assert_eq!(row.id(), &RowId::Group(ChangeGroup::Changes));
    }

    #[test]
    fn build_rows_creates_shared_prefix_root() {
        let files = vec![
            file("src/ui/mod.rs", 3, 1),
            file("src/ui/header.rs", 4, 0),
            file("src/ui/help_overlay.rs", 6, 2),
        ];

        let rows = build_visible_rows(&files, &["src/ui".to_string()].into_iter().collect());

        assert_eq!(rows[0].label(), "src/ui/");
        assert!(rows[0].is_directory());
        assert_eq!(rows[1].label(), "header.rs");
        assert_eq!(rows[2].label(), "help_overlay.rs");
        assert_eq!(rows[3].label(), "mod.rs");
    }

    #[test]
    fn build_rows_respects_collapsed_directories() {
        let files = vec![file("src/app.rs", 1, 0), file("src/ui/mod.rs", 3, 1)];

        let rows = build_visible_rows(&files, &std::collections::BTreeSet::new());

        assert_eq!(
            rows.iter().map(|row| row.label()).collect::<Vec<_>>(),
            vec!["src/"]
        );
    }

    #[test]
    fn build_rows_uses_full_path_as_stable_file_id() {
        let files = vec![file("src/ui/mod.rs", 3, 1)];
        let rows = build_visible_rows(
            &files,
            &["src".to_string(), "src/ui".to_string()]
                .into_iter()
                .collect(),
        );

        assert_eq!(rows[2].id(), &RowId::File("src/ui/mod.rs".to_string()));
    }
}
