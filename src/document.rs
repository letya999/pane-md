use std::{
    borrow::Cow,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use super::Result;
use super::i18n::{Label as L, tr};
use pane_md::Bytes;
pub use pane_md::links::markdown_path;
#[cfg(windows)]
use windows::{
    Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, REPLACE_FILE_FLAGS, ReplaceFileW,
    },
    core::PCWSTR,
};

pub struct Document {
    pub id: usize,
    location: Location,
    pub text: String,
    pub bom: bool,
    saved: Vec<u8>,
    undo: Vec<Change>,
    redo: Vec<Change>,
    #[cfg(windows)]
    pub viewport: Viewport,
    #[cfg(windows)]
    pub mode_round_trip: Option<(Viewport, Viewport)>,
    #[cfg(windows)]
    pub horizontal_scroll: [i32; 2],
    pub mode: usize,
    pub show_reviews: bool,
}

pub struct LinkPosition<V> {
    pub id: usize,
    pub path: Option<PathBuf>,
    pub mode: usize,
    pub viewport: V,
}

impl<V> LinkPosition<V> {
    pub fn open_tab(&self, docs: &[Document]) -> Option<usize> {
        docs.iter().position(|doc| doc.id == self.id).or_else(|| {
            let path = self.path.as_deref()?;
            docs.iter().position(|doc| doc.path() == Some(path))
        })
    }
}

pub struct LinkHistory<V> {
    back: Vec<LinkPosition<V>>,
    forward: Vec<LinkPosition<V>>,
}

impl<V> Default for LinkHistory<V> {
    fn default() -> Self {
        Self {
            back: Vec::new(),
            forward: Vec::new(),
        }
    }
}

impl<V> LinkHistory<V> {
    pub fn visit(&mut self, from: LinkPosition<V>) {
        self.back.push(from);
        self.forward.clear();
    }

    pub fn available(&self, forward: bool) -> bool {
        !(if forward { &self.forward } else { &self.back }).is_empty()
    }

    pub fn pop(&mut self, forward: bool) -> Option<LinkPosition<V>> {
        (if forward {
            &mut self.forward
        } else {
            &mut self.back
        })
        .pop()
    }

    pub fn leave(&mut self, forward: bool, from: LinkPosition<V>) {
        (if forward {
            &mut self.back
        } else {
            &mut self.forward
        })
        .push(from);
    }

    pub fn saved_as(&mut self, document: &Document, previous: Option<&Path>) {
        let Some(path) = document.path() else { return };
        for position in self.back.iter_mut().chain(&mut self.forward) {
            if position.id == document.id
                || previous.is_some() && position.path.as_deref() == previous
            {
                position.id = document.id;
                position.path = Some(path.to_owned());
            }
        }
    }
}

enum Location {
    File(PathBuf),
    Untitled(usize),
}

#[cfg(windows)]
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Viewport {
    pub selection: Bytes,
    pub top: usize,
    pub line_y: i32,
}

struct Change {
    start: usize,
    before: String,
    after: String,
}

fn decode(bytes: &[u8]) -> Result<(bool, &str)> {
    pane_md::text_file::decode(bytes).map_err(|error| match error {
        pane_md::text_file::DecodeError::Nul => tr(L::NullInText).into(),
        error => error.into(),
    })
}

pub struct FolderEntry {
    pub path: PathBuf,
    pub directory: bool,
}

pub fn folder_entries(path: &Path) -> io::Result<Vec<FolderEntry>> {
    let mut entries = fs::read_dir(path)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let kind = entry.file_type().ok()?;
            let path = entry.path();
            let directory = kind.is_dir();
            (directory || kind.is_file() && markdown_path(&path))
                .then_some(FolderEntry { path, directory })
        })
        .collect::<Vec<_>>();
    entries.sort_by_cached_key(|entry| {
        (
            !entry.directory,
            entry
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase(),
            entry.path.file_name().unwrap_or_default().to_os_string(),
        )
    });
    Ok(entries)
}

impl Document {
    pub fn link_position<V>(&self, viewport: V) -> LinkPosition<V> {
        LinkPosition {
            id: self.id,
            path: self.path().map(Path::to_owned),
            mode: self.mode,
            viewport,
        }
    }

    pub fn blank() -> Self {
        static UNTITLED: AtomicUsize = AtomicUsize::new(1);
        let number = UNTITLED.fetch_add(1, Ordering::Relaxed);
        Self::new(
            Location::Untitled(number),
            String::new(),
            Vec::new(),
            false,
            2,
        )
    }

    pub fn open(path: PathBuf, mode: usize) -> Result<Self> {
        let saved = fs::read(&path)?;
        let (bom, text) = decode(&saved)?;
        Ok(Self::new(
            Location::File(path),
            text.to_owned(),
            saved,
            bom,
            mode,
        ))
    }

    fn new(location: Location, text: String, saved: Vec<u8>, bom: bool, mode: usize) -> Self {
        static IDS: AtomicUsize = AtomicUsize::new(1);
        Self {
            id: IDS.fetch_add(1, Ordering::Relaxed),
            location,
            text,
            bom,
            saved,
            mode,
            show_reviews: true,
            undo: Vec::new(),
            redo: Vec::new(),
            #[cfg(windows)]
            viewport: Viewport::default(),
            #[cfg(windows)]
            mode_round_trip: None,
            #[cfg(windows)]
            horizontal_scroll: [0; 2],
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.location {
            Location::File(path) => Some(path),
            Location::Untitled(_) => None,
        }
    }

    pub fn name(&self) -> Cow<'_, str> {
        match &self.location {
            Location::File(path) => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy(),
            Location::Untitled(number) => format!("{} {number}", tr(L::Untitled)).into(),
        }
    }

    pub fn suggested_filename(&self) -> String {
        if self.path().is_some() {
            self.name().into_owned()
        } else {
            format!("{}.md", self.name())
        }
    }

    pub fn resolve_link(&self, target: &Path) -> Result<PathBuf> {
        if target.is_absolute() {
            return Ok(target.canonicalize()?);
        }
        let path = self.path().ok_or(tr(L::SaveBeforeRelative))?;
        let folder = path.parent().ok_or(tr(L::MissingDirectory))?;
        Ok(folder.join(target).canonicalize()?)
    }

    pub fn dirty(&self) -> bool {
        let bytes = if self.bom {
            &self.saved[3..]
        } else {
            &self.saved
        };
        bytes != self.text.as_bytes()
    }

    pub fn patch(&mut self, range: Bytes, replacement: &str, coalesce: bool) {
        let before = &self.text[range.clone()];
        if before == replacement {
            return;
        }
        #[cfg(windows)]
        {
            self.mode_round_trip = None;
        }
        let start = range.start;
        let mut merged = false;
        if coalesce
            && before.is_empty()
            && !replacement.contains(['\r', '\n'])
            && let Some(prev) = self.undo.last_mut()
            && prev.before.is_empty()
            && prev.start + prev.after.len() == start
            && !prev.after.contains(['\r', '\n'])
            && prev.after.len() < 128
        {
            prev.after.push_str(replacement);
            merged = true;
        }
        if !merged {
            self.undo.push(Change {
                start,
                before: before.into(),
                after: replacement.into(),
            });
        }
        self.redo.clear();
        self.text.replace_range(range, replacement);
    }

    pub fn replace_source(
        &mut self,
        query: &str,
        replacement: &str,
        options: pane_md::search::Options,
        selected: Option<Bytes>,
    ) -> Option<(usize, usize)> {
        if self.mode != 3 {
            return None;
        }
        let edit = pane_md::search::replace(&self.text, query, replacement, options, selected)?;
        self.patch(edit.range, &edit.text, false);
        Some((edit.caret, edit.count))
    }

    pub fn undo(&mut self, redo: bool) -> Option<usize> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let change = from.pop()?;
        #[cfg(windows)]
        {
            self.mode_round_trip = None;
        }
        let (old, new) = if redo {
            (&change.before, &change.after)
        } else {
            (&change.after, &change.before)
        };
        self.text
            .replace_range(change.start..change.start + old.len(), new);
        let pos = change.start + new.len();
        to.push(change);
        Some(pos)
    }

    pub fn save(&mut self) -> Result<()> {
        let path = self.path().ok_or(tr(L::SaveBeforeRelative))?;
        if !self.dirty() {
            return Ok(());
        }
        let parent = path.parent().ok_or(tr(L::NoParent))?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        let mut bytes = Vec::with_capacity(self.text.len() + 3);
        if self.bom {
            bytes.extend_from_slice(b"\xef\xbb\xbf");
        }
        bytes.extend_from_slice(self.text.as_bytes());
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        replace_file(path, temp, &self.saved)?;
        self.saved = bytes;
        Ok(())
    }

    pub fn save_as(&mut self, path: PathBuf) -> Result<()> {
        if let Some(current) = self.path()
            && (path == current || path.canonicalize().is_ok_and(|path| path == current))
        {
            return self.save();
        }
        let parent = path.parent().ok_or(tr(L::NoParent))?.canonicalize()?;
        let path = parent.join(path.file_name().ok_or(tr(L::NoParent))?);
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        let mut bytes = Vec::with_capacity(self.text.len() + 3);
        if self.bom {
            bytes.extend_from_slice(b"\xef\xbb\xbf");
        }
        bytes.extend_from_slice(self.text.as_bytes());
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        // The dialog chooses a new filename. A file that appeared in the meantime is kept.
        temp.persist_noclobber(&path)?;
        self.location = Location::File(path);
        self.saved = bytes;
        Ok(())
    }
}

#[cfg(windows)]
fn replace_file(path: &Path, temp: tempfile::NamedTempFile, expected: &[u8]) -> Result<()> {
    use std::{
        fs::OpenOptions,
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    };
    // Deny new writers when possible. The terminal editor may already hold a writable handle.
    let conflict = || io::Error::other(tr(L::FileChanged));
    let mut current = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_DELETE.0)
        .open(path)
        .or_else(|error| {
            if error.raw_os_error() == Some(32) {
                fs::File::open(path)
            } else {
                Err(error)
            }
        })
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                conflict()
            } else {
                error
            }
        })?;
    let mut on_disk = Vec::new();
    current.read_to_end(&mut on_disk)?;
    if on_disk != expected {
        return Err(conflict().into());
    }
    let as_wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let saved = as_wide(path);
    let temporary_path = temp.into_temp_path();
    let replacement = as_wide(&temporary_path);
    unsafe {
        ReplaceFileW(
            PCWSTR(saved.as_ptr()),
            PCWSTR(replacement.as_ptr()),
            PCWSTR::null(),
            REPLACE_FILE_FLAGS(0),
            None,
            None,
        )?;
    }
    Ok(())
}

#[cfg(unix)]
fn replace_file(path: &Path, temp: tempfile::NamedTempFile, expected: &[u8]) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let conflict = || io::Error::other(tr(L::FileChanged));
    let same_file = |a: &fs::Metadata, b: &fs::Metadata| {
        a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.len() == b.len()
            && a.modified().ok() == b.modified().ok()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    };
    let before = fs::symlink_metadata(path).map_err(|_| conflict())?;
    if !before.is_file() {
        return Err(conflict().into());
    }
    let mut current = fs::File::open(path)?;
    if !same_file(&before, &current.metadata()?) {
        return Err(conflict().into());
    }
    let mut on_disk = Vec::new();
    current.read_to_end(&mut on_disk)?;
    if on_disk != expected {
        return Err(conflict().into());
    }
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(before.mode() & 0o777))?;
    temp.as_file().sync_all()?;
    if !same_file(
        &before,
        &fs::symlink_metadata(path).map_err(|_| conflict())?,
    ) {
        return Err(conflict().into());
    }
    temp.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_note_keeps_its_text_and_identity_until_first_save_succeeds() -> Result<()> {
        let destination = tempfile::tempdir()?;
        let existing = destination.path().join("Existing.md");
        fs::write(&existing, "Other note")?;
        let mut note = Document::blank();
        let id = note.id;
        assert!(note.path().is_none() && !note.dirty());
        assert!(note.suggested_filename().ends_with(".md"));
        assert!(note.resolve_link(Path::new("Existing.md")).is_err());
        note.patch(0..0, "# План 😀\n", false);
        let mut history = LinkHistory::default();
        history.visit(note.link_position(4));
        assert!(note.dirty());
        assert!(note.save_as(existing.clone()).is_err());
        assert_eq!(fs::read_to_string(existing)?, "Other note");
        assert!(note.dirty() && note.path().is_none());

        let saved = destination.path().join("First note.md");
        note.save_as(saved.clone())?;
        history.saved_as(&note, None);
        let position = history.pop(false).unwrap();
        assert_eq!(position.path.as_deref(), note.path());
        assert_eq!(position.viewport, 4);
        assert_eq!(note.id, id);
        assert_eq!(note.path(), Some(saved.canonicalize()?.as_path()));
        assert_eq!(fs::read_to_string(saved)?, "# План 😀\n");
        assert!(!note.dirty());
        assert!(note.undo(false).is_some() && note.dirty());
        Ok(())
    }

    #[test]
    fn save_as_resolves_file_aliases_and_preserves_existing_targets() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().canonicalize()?;
        let alias = directory.path().join("sub").join("..");
        fs::create_dir(root.join("sub"))?;
        let first = root.join("Plan.md");
        let second = root.join("Copy.md");
        fs::write(&first, "old\r\n")?;
        let mut doc = Document::open(first.clone(), 3)?;
        doc.patch(0..3, "new", false);
        doc.save_as(alias.join("Plan.md"))?;
        assert_eq!(doc.path(), Some(first.as_path()));
        assert_eq!(fs::read_to_string(&first)?, "new\r\n");
        doc.save_as(alias.join("Copy.md"))?;
        assert_eq!(doc.path(), Some(second.canonicalize()?.as_path()));
        assert_eq!(fs::read_to_string(&second)?, "new\r\n");
        doc.patch(0..3, "unsaved", false);
        assert!(doc.save_as(alias.join("Plan.md")).is_err());
        assert_eq!(doc.path(), Some(second.as_path()));
        assert!(doc.dirty());
        assert_eq!(fs::read_to_string(first)?, "new\r\n");
        assert_eq!(fs::read_to_string(second)?, "new\r\n");
        Ok(())
    }
}
