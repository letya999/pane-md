use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::windows::ffi::OsStrExt,
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use super::Result;
use notepad_md::Bytes;
use windows::{
    Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, REPLACE_FILE_FLAGS, ReplaceFileW,
    },
    core::PCWSTR,
};

pub struct Document {
    pub path: PathBuf,
    pub text: String,
    pub bom: bool,
    saved: Vec<u8>,
    undo: Vec<Change>,
    redo: Vec<Change>,
    pub position: usize,
    pub mode: usize,
}

struct Change {
    start: usize,
    before: String,
    after: String,
}

pub fn decode(bytes: &[u8]) -> Result<(bool, &str)> {
    let bom = bytes.starts_with(b"\xef\xbb\xbf");
    let text = std::str::from_utf8(if bom { &bytes[3..] } else { bytes })?;
    if text.contains('\0') {
        return Err("в тексте обнаружен нулевой символ".into());
    }
    Ok((bom, text))
}

pub fn markdown_path(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        ["md", "markdown", "mdown", "mkd"]
            .iter()
            .any(|ext| s.eq_ignore_ascii_case(ext))
    })
}

impl Document {
    pub fn open(path: PathBuf, mode: usize) -> Result<Self> {
        let saved = fs::read(&path)?;
        let (bom, text) = decode(&saved)?;
        Ok(Self {
            path,
            text: text.to_owned(),
            bom,
            saved,
            mode,
            undo: Vec::new(),
            redo: Vec::new(),
            position: 0,
        })
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

    pub fn undo(&mut self, redo: bool) -> Option<usize> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let change = from.pop()?;
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
        if !self.dirty() {
            return Ok(());
        }
        let parent = self.path.parent().ok_or("у файла нет родительской папки")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        let mut bytes = Vec::with_capacity(self.text.len() + 3);
        if self.bom {
            bytes.extend_from_slice(b"\xef\xbb\xbf");
        }
        bytes.extend_from_slice(self.text.as_bytes());
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        // Deny new writers when possible. The terminal editor may already hold a writable handle.
        let conflict = || {
            io::Error::other(
                "Файл изменён на диске. Сохраните буфер под другим именем через «Сохранить как».",
            )
        };
        let mut current = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_DELETE.0)
            .open(&self.path)
            .or_else(|error| {
                if error.raw_os_error() == Some(32) {
                    fs::File::open(&self.path)
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
        if on_disk != self.saved {
            return Err(conflict().into());
        }
        let as_wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        let saved = as_wide(&self.path);
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
        self.saved = bytes;
        Ok(())
    }

    pub fn save_as(&mut self, path: PathBuf) -> Result<()> {
        if path == self.path {
            return self.save();
        }
        let mut temp = tempfile::NamedTempFile::new_in(
            path.parent().ok_or("у файла нет родительской папки")?,
        )?;
        let mut bytes = Vec::with_capacity(self.text.len() + 3);
        if self.bom {
            bytes.extend_from_slice(b"\xef\xbb\xbf");
        }
        bytes.extend_from_slice(self.text.as_bytes());
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        // The dialog chooses a new filename. A file that appeared in the meantime is kept.
        temp.persist_noclobber(&path)?;
        self.path = path;
        self.saved = bytes;
        Ok(())
    }
}
