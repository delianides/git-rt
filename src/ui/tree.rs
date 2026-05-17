use std::collections::{BTreeMap, BTreeSet};

use crate::git::FileEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowId {
    Directory(String),
    File(String),
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
}

impl VisibleRow {
    pub fn id(&self) -> &RowId {
        match self {
            VisibleRow::Directory { id, .. } | VisibleRow::File { id, .. } => id,
        }
    }

    pub fn depth(&self) -> usize {
        match self {
            VisibleRow::Directory { depth, .. } | VisibleRow::File { depth, .. } => *depth,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            VisibleRow::Directory { label, .. } | VisibleRow::File { label, .. } => label,
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self, VisibleRow::Directory { .. })
    }

    pub fn directory_expanded(&self) -> Option<bool> {
        match self {
            VisibleRow::Directory { expanded, .. } => Some(*expanded),
            VisibleRow::File { .. } => None,
        }
    }

    pub fn file(&self) -> Option<&FileEntry> {
        match self {
            VisibleRow::Directory { .. } => None,
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
