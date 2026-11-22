use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;

use crate::document::{FolderEntry, folder_entries};

pub(super) struct FileTree {
    pub path: PathBuf,
    nodes: Vec<Node>,
}

struct Node {
    entry: FolderEntry,
    branch: Option<Branch>,
}

#[derive(Default)]
struct Branch {
    expanded: bool,
    children: Vec<Node>,
}

#[derive(Serialize)]
pub(super) struct Row {
    id: String,
    name: String,
    depth: usize,
    expanded: Option<bool>,
    current: bool,
}

impl FileTree {
    pub fn open(path: PathBuf) -> io::Result<Self> {
        let nodes = reconcile(folder_entries(&path)?, Vec::new());
        Ok(Self { path, nodes })
    }

    pub fn refresh(&mut self) -> io::Result<()> {
        let entries = folder_entries(&self.path)?;
        self.nodes = reconcile(entries, std::mem::take(&mut self.nodes));
        Ok(())
    }

    pub fn activate(&mut self, mut index: usize) -> io::Result<Option<PathBuf>> {
        let Some(node) = node_at(&mut self.nodes, &mut index) else {
            return Ok(None);
        };
        let Some(branch) = &mut node.branch else {
            return Ok(Some(node.entry.path.clone()));
        };
        if branch.expanded {
            branch.expanded = false;
        } else {
            let entries = folder_entries(&node.entry.path)?;
            branch.children = reconcile(entries, std::mem::take(&mut branch.children));
            branch.expanded = true;
        }
        Ok(None)
    }

    pub fn rows(&self, active: Option<&Path>) -> Vec<Row> {
        let mut rows = Vec::new();
        collect_rows(&self.nodes, active, 0, &mut rows);
        rows
    }
}

fn reconcile(entries: Vec<FolderEntry>, previous: Vec<Node>) -> Vec<Node> {
    let mut branches: HashMap<_, _> = previous
        .into_iter()
        .filter_map(|node| Some((node.entry.path, node.branch?)))
        .collect();
    entries
        .into_iter()
        .map(|entry| {
            let branch = entry.directory.then(|| {
                let mut branch = branches.remove(&entry.path).unwrap_or_default();
                if branch.expanded {
                    match folder_entries(&entry.path) {
                        Ok(entries) => branch.children = reconcile(entries, branch.children),
                        Err(_) => branch = Branch::default(),
                    }
                }
                branch
            });
            Node { entry, branch }
        })
        .collect()
}

fn node_at<'a>(nodes: &'a mut [Node], index: &mut usize) -> Option<&'a mut Node> {
    for node in nodes {
        if *index == 0 {
            return Some(node);
        }
        *index -= 1;
        if let Some(branch) = &mut node.branch
            && branch.expanded
            && let Some(found) = node_at(&mut branch.children, index)
        {
            return Some(found);
        }
    }
    None
}

fn collect_rows(nodes: &[Node], active: Option<&Path>, depth: usize, rows: &mut Vec<Row>) {
    for node in nodes {
        let path = node.entry.path.as_path();
        rows.push(Row {
            id: URL_SAFE_NO_PAD.encode(path.as_os_str().as_encoded_bytes()),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            depth,
            expanded: node.branch.as_ref().map(|branch| branch.expanded),
            current: active == Some(path),
        });
        if let Some(branch) = &node.branch
            && branch.expanded
        {
            collect_rows(&branch.children, active, depth + 1, rows);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn folders_open_lazily_and_refresh_preserves_the_visible_tree() -> io::Result<()> {
        let folder = tempfile::tempdir()?;
        let root = folder.path();
        let archive = root.join("archive");
        let inbox = root.join("inbox");
        let year = archive.join("2026");
        let note = year.join("План #1.MD");
        fs::create_dir_all(&year)?;
        fs::create_dir(&inbox)?;
        fs::write(&note, "# План")?;
        fs::write(inbox.join("План #1.MD"), "# Другая заметка")?;
        fs::write(inbox.join("ignored.txt"), "not Markdown")?;
        let mut tree = FileTree::open(root.to_owned())?;
        assert_eq!(tree.rows(None).len(), 2);
        assert!(tree.nodes[0].branch.as_ref().unwrap().children.is_empty());
        assert_eq!(tree.activate(0)?, None);
        assert_eq!(tree.activate(1)?, None);
        assert_eq!(tree.activate(3)?, None);
        assert_eq!(tree.activate(2)?.as_deref(), Some(note.as_path()));
        let rows = tree.rows(Some(&note));
        assert_eq!(rows.len(), 5);
        assert!(rows[2].current);
        assert_eq!(rows[2].depth, 2);
        assert_eq!(rows[4].depth, 1);
        assert_ne!(rows[2].id, rows[4].id);

        fs::write(root.join("draft.md"), "# Draft")?;
        fs::write(year.join("new.md"), "# New")?;
        fs::remove_file(&note)?;
        tree.refresh()?;
        let rows = tree.rows(None);
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            [
                "archive",
                "2026",
                "new.md",
                "inbox",
                "План #1.MD",
                "draft.md"
            ]
        );
        for i in [0, 1, 3] {
            assert_eq!(rows[i].expanded, Some(true));
        }
        tree.activate(0)?;
        assert_eq!(tree.rows(None).len(), 4);
        tree.activate(0)?;
        assert_eq!(tree.rows(None)[1].expanded, Some(true));
        assert_eq!(tree.activate(2)?, Some(year.join("new.md")));
        assert_eq!(tree.activate(100)?, None);
        Ok(())
    }
}
