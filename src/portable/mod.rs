mod appearance;
mod diagrams;
mod dialog;
mod files;
mod format;
mod navigation;
mod preview;
mod reviews;

use std::{
    ffi::OsStr,
    fs, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use pane_md::{
    diff, edit_source,
    links::{Destination, TargetError, destination},
    markdown::Markdown,
    review::Binding,
    source_view, utf16_byte,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::{Icon, Window, WindowBuilder},
};
use wry::{
    WebViewBuilder,
    http::{Request, Response, StatusCode, Uri},
};

use crate::{
    Result,
    document::{Document, LinkHistory, markdown_path},
    i18n::{self, Label as L, tr},
};
use dialog::{Pick, SaveChoice};
use files::FileTree;

const ORIGIN: &str = "panemd://app/";
const ICON: &[u8] = include_bytes!("../../assets/pane-md.png");

#[derive(Deserialize)]
struct Message {
    #[serde(flatten)]
    action: Action,
    source: Option<String>,
    #[serde(default)]
    edits: Vec<RichEdit>,
}

#[derive(Deserialize)]
struct RichEdit {
    region: usize,
    start: usize,
    end: usize,
    text: String,
}

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum Action {
    Ready {
        system_dark: bool,
    },
    SystemAppearance {
        dark: bool,
    },
    AppearanceFonts,
    SetEditorAppearance {
        appearance: appearance::Appearance,
    },
    DiagramSource {
        region: usize,
    },
    FindHeading {
        query: String,
        previous: bool,
        caret: usize,
        region: Option<usize>,
    },
    ShowInSource {
        region: Option<usize>,
        selection: Option<[reviews::Part; 2]>,
    },
    OpenFile,
    OpenFolder,
    NewNote,
    RefreshFiles,
    FolderUp,
    Browse {
        index: usize,
    },
    SelectTab {
        index: usize,
    },
    CloseTab {
        index: usize,
    },
    Family {
        source_view: bool,
    },
    Stage {
        stage: Stage,
    },
    QuickMode {
        mode: usize,
    },
    Save,
    SaveAs,
    Undo {
        redo: bool,
    },
    Lint {
        previous: bool,
        caret: Option<usize>,
        region: Option<usize>,
    },
    Link {
        target: String,
        viewport: Option<LinkViewport>,
    },
    SourceLink {
        caret: usize,
        viewport: Option<LinkViewport>,
    },
    LinkHistory {
        forward: bool,
        viewport: Option<LinkViewport>,
    },
    Format {
        #[serde(flatten)]
        action: format::Command,
        selection: reviews::Part,
    },
    ReplaceSource {
        query: String,
        replacement: String,
        options: pane_md::search::Options,
        selection: Option<[usize; 2]>,
    },
    InsertImage {
        selection: Option<reviews::Part>,
    },
    ReplaceVisual {
        selection: Vec<reviews::Part>,
        text: String,
    },
    ToggleTask {
        region: usize,
    },
    Review {
        #[serde(flatten)]
        operation: reviews::Operation,
    },
    OpenDropped,
    Theme {
        theme: Theme,
    },
    Language {
        english: bool,
    },
    Exit,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Read,
    Comments,
    Edit,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Default, Deserialize, Serialize)]
struct Preferences {
    english: bool,
    theme: Theme,
    #[serde(default)]
    appearance: appearance::Appearance,
}

impl Preferences {
    fn path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        #[cfg(target_os = "macos")]
        let config = home?.join("Library/Application Support");
        #[cfg(target_os = "linux")]
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".config")))?;
        Some(config.join("pane-md").join("settings.json"))
    }

    fn read() -> Self {
        Self::path()
            .and_then(|path| Self::load_with_legacy(&path))
            .unwrap_or_else(|| Self {
                english: !["LC_ALL", "LC_MESSAGES", "LANG"]
                    .iter()
                    .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
                    .is_some_and(|locale| locale.starts_with("ru")),
                ..Self::default()
            })
    }

    fn load_with_legacy(path: &Path) -> Option<Self> {
        Self::load(path).or_else(|| {
            let old = path.parent()?.parent()?.join("notepad-md/settings.json");
            Self::load(&old)
        })
    }

    fn load(path: &Path) -> Option<Self> {
        let mut preferences: Self = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        preferences.appearance.sanitize();
        Some(preferences)
    }

    fn write(&self) -> Result<()> {
        if let Some(path) = Self::path() {
            self.write_to(&path)?;
        }
        Ok(())
    }

    fn write_to(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let dir = path.parent().ok_or(tr(L::MissingDirectory))?;
        fs::create_dir_all(dir)?;
        let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
        temporary.write_all(&serde_json::to_vec(self)?)?;
        temporary.persist(path)?;
        Ok(())
    }

    fn set_appearance(&mut self, appearance: appearance::Appearance) -> Result<()> {
        appearance.validate()?;
        let previous = std::mem::replace(&mut self.appearance, appearance);
        if let Err(error) = self.write() {
            self.appearance = previous;
            return Err(error);
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkViewport {
    scroll_top: f64,
    scroll_left: f64,
    selection: [usize; 2],
    visual_selection: Option<[[usize; 2]; 2]>,
    jump_region: Option<usize>,
}

#[derive(Default)]
struct Workspace {
    docs: Vec<Document>,
    active: Option<usize>,
    folder: Option<FileTree>,
    history: LinkHistory<Option<LinkViewport>>,
    link_viewport: Option<LinkViewport>,
    jump: Option<usize>,
    jump_end: Option<usize>,
    editor_selection: Option<format::CaretTarget>,
    system_dark: bool,
    diagrams: diagrams::View,
    notice: String,
    preferences: Preferences,
    wait: bool,
    dropped: Vec<PathBuf>,
    reviews: reviews::Editor,
}

impl Workspace {
    fn open(&mut self, path: PathBuf, mode: usize) -> Result<()> {
        let path = path.canonicalize()?;
        if path.is_dir() {
            if self.wait {
                return Err(tr(L::WaitNeedsFile).into());
            }
            return self.set_folder(path);
        }
        if let Some(index) = self
            .docs
            .iter()
            .position(|doc| doc.path() == Some(path.as_path()))
        {
            self.active = Some(index);
        } else if !self.wait || self.docs.is_empty() {
            if !path.is_file() {
                return Err(tr(L::FileError).into());
            }
            self.docs.push(Document::open(path, mode)?);
            self.active = Some(self.docs.len() - 1);
        }
        Ok(())
    }

    fn set_folder(&mut self, path: PathBuf) -> Result<()> {
        if let Some(folder) = &mut self.folder
            && folder.path == path
        {
            folder.refresh()?;
        } else {
            self.folder = Some(FileTree::open(path)?);
        }
        Ok(())
    }

    fn apply_edits(doc: &mut Document, source: Option<String>, edits: Vec<RichEdit>) -> Result<()> {
        if doc.mode == 3
            && let Some(source) = source
        {
            if source.contains('\0') {
                return Err(tr(L::NullInText).into());
            }
            let (before, map) = source_view(&doc.text);
            let after: Vec<u16> = source.replace('\n', "\r").encode_utf16().collect();
            if before != after {
                let (range, text) =
                    edit_source(&doc.text, &before, &map, &after).ok_or(tr(L::UnsupportedEdit))?;
                doc.patch(range, &text, false);
            }
        }
        if doc.mode == 2 && !edits.is_empty() {
            let parsed = format::projection(doc);
            let mut patches = edits
                .into_iter()
                .map(|edit| {
                    let region = parsed.regions.get(edit.region)?;
                    if edit.start > edit.end || edit.text.contains('\0') {
                        return None;
                    }
                    let old = region.text.get(
                        utf16_byte(&region.text, edit.start)?..utf16_byte(&region.text, edit.end)?,
                    )?;
                    let old: Vec<_> = old
                        .chars()
                        .map(|ch| if ch == '\r' { '\n' } else { ch })
                        .collect();
                    let new: Vec<_> = edit.text.chars().collect();
                    let (removed, inserted) = diff(&old, &new);
                    let units = |end| {
                        edit.start + old[..end].iter().map(|ch| ch.len_utf16()).sum::<usize>()
                    };
                    region
                        .patch(
                            &doc.text,
                            units(removed.start)..units(removed.end),
                            &new[inserted].iter().collect::<String>(),
                        )
                        .map(|(range, text)| (range, text, edit))
                })
                .collect::<Option<Vec<_>>>()
                .ok_or(tr(L::UnsupportedEdit))?;
            patches.sort_by_key(|(range, _, edit)| (std::cmp::Reverse(range.start), edit.region));
            if patches
                .windows(2)
                .any(|pair| pair[0].0.start < pair[1].0.end)
            {
                return Err(tr(L::UnsupportedEdit).into());
            }
            let combined = patches
                .chunk_by(|left, right| left.0.start == right.0.start)
                .map(|group| {
                    if let [(range, text, _)] = group {
                        return Some((range.clone(), text.clone()));
                    }
                    if group.iter().any(|(range, _, edit)| {
                        !range.is_empty() || edit.start != 0 || edit.end != 0
                    }) {
                        return None;
                    }
                    let edits: Vec<_> = group
                        .iter()
                        .map(|(_, _, edit)| (edit.region, edit.text.as_str()))
                        .collect();
                    parsed.fill_empty_cells(&doc.text, &edits)
                })
                .collect::<Option<Vec<_>>>()
                .ok_or(tr(L::UnsupportedEdit))?;
            for (range, text) in combined {
                doc.patch(range, &text, false);
            }
        }
        Ok(())
    }

    fn active_doc(&mut self) -> Option<&mut Document> {
        self.active.and_then(|index| self.docs.get_mut(index))
    }

    fn confirms(&mut self, index: usize, window: &Window) -> Result<bool> {
        let Some(doc) = self.docs.get(index) else {
            return Ok(false);
        };
        if !doc.dirty() {
            return Ok(true);
        }
        let question = tr(L::ConfirmSave).replace("{file}", &doc.name());
        match dialog::confirm(window, &question) {
            SaveChoice::Save => self.save(index, window, false),
            SaveChoice::Discard => Ok(true),
            SaveChoice::Cancel => Ok(false),
        }
    }

    fn save(&mut self, index: usize, window: &Window, save_as: bool) -> Result<bool> {
        let doc = &mut self.docs[index];
        if save_as || doc.path().is_none() {
            let name = doc.suggested_filename();
            let previous_file = doc.path().map(Path::to_owned);
            let previous = doc
                .path()
                .or(self.folder.as_ref().map(|folder| folder.path.as_path()));
            let Some(path) = dialog::path(window, Pick::SaveAs(&name), previous) else {
                return Ok(false);
            };
            doc.save_as(path)?;
            self.history.saved_as(doc, previous_file.as_deref());
            if let Some(folder) = self.folder.as_mut()
                && doc
                    .path()
                    .is_some_and(|path| path.starts_with(&folder.path))
            {
                let _ = folder.refresh();
            }
        } else {
            doc.save()?;
        }
        Ok(true)
    }

    fn act(&mut self, action: Action, window: &Window) -> Result<bool> {
        self.notice.clear();
        self.jump = None;
        self.jump_end = None;
        self.link_viewport = None;
        self.editor_selection = None;
        match action {
            Action::Ready { system_dark } => self.system_dark = system_dark,
            Action::NewNote if !self.wait => {
                self.docs.push(Document::blank());
                self.active = Some(self.docs.len() - 1);
                self.editor_selection = Some(format::CaretTarget::Source(0..0));
            }
            Action::DiagramSource { region } => {
                if let Some(doc) = self.active_doc()
                    && let Some(region) = Markdown::parse(&doc.text).regions.into_iter().nth(region)
                    && region.mermaid
                {
                    doc.mode |= 1;
                    self.jump = Some(region.source.start);
                    self.jump_end = Some(region.source.end);
                }
            }
            Action::FindHeading {
                query,
                previous,
                caret,
                region,
            } => {
                self.find_heading(&query, previous, caret, region);
            }
            Action::ShowInSource { region, selection } => self.show_in_source(region, selection),
            Action::OpenFile | Action::OpenFolder if !self.wait => {
                let pick = if matches!(action, Action::OpenFile) {
                    Pick::File
                } else {
                    Pick::Folder
                };
                let previous = self
                    .folder
                    .as_ref()
                    .map(|folder| folder.path.as_path())
                    .or_else(|| self.active.and_then(|index| self.docs[index].path()));
                if let Some(path) = dialog::path(window, pick, previous) {
                    self.open(path, 0)?;
                }
            }
            Action::FolderUp if !self.wait => {
                let parent = self.folder.as_ref().and_then(|folder| folder.path.parent());
                if let Some(parent) = parent {
                    self.set_folder(parent.to_owned())?;
                }
            }
            Action::RefreshFiles if !self.wait => {
                if let Some(folder) = &mut self.folder {
                    folder.refresh()?;
                }
            }
            Action::Browse { index } if !self.wait => {
                if let Some(folder) = &mut self.folder
                    && let Some(path) = folder.activate(index)?
                {
                    self.open(path, 0)?;
                }
            }
            Action::SelectTab { index } => {
                if index < self.docs.len() {
                    self.active = Some(index);
                }
            }
            Action::CloseTab { index } => {
                if self.confirms(index, window)? {
                    if self.wait {
                        return Ok(true);
                    }
                    self.docs.remove(index);
                    self.active = if self.docs.is_empty() {
                        None
                    } else {
                        self.active.map(|active| {
                            if active > index {
                                active - 1
                            } else {
                                active.min(self.docs.len() - 1)
                            }
                        })
                    };
                }
            }
            Action::Family { source_view } => {
                if let Some(doc) = self.active_doc() {
                    doc.mode = doc.mode & !1 | usize::from(source_view);
                }
            }
            Action::Stage { stage } => {
                if let Some(doc) = self.active_doc() {
                    doc.mode = doc.mode & 1 | if matches!(stage, Stage::Edit) { 2 } else { 0 };
                    doc.show_reviews = !matches!(stage, Stage::Read);
                }
            }
            Action::QuickMode { mode } if mode <= 3 => {
                if let Some(doc) = self.active_doc() {
                    doc.mode = mode;
                    doc.show_reviews = mode >= 2;
                }
            }
            Action::Save => {
                if let Some(index) = self.active
                    && self.save(index, window, false)?
                {
                    self.notice = tr(L::Saved).into();
                }
            }
            Action::SaveAs if !self.wait => {
                if let Some(index) = self.active
                    && self.save(index, window, true)?
                {
                    self.notice = tr(L::Saved).into();
                }
            }
            Action::Undo { redo } => {
                if let Some(doc) = self.active_doc() {
                    let at = doc.undo(redo);
                    if doc.mode == 2 {
                        self.editor_selection = at.map(|at| format::CaretTarget::Source(at..at));
                    } else {
                        self.jump = at;
                    }
                }
            }
            Action::Lint {
                previous,
                caret,
                region,
            } => self.next_lint(previous, caret, region),
            Action::Link { target, viewport } => self.follow(&target, viewport)?,
            Action::SourceLink { caret, viewport } => {
                if let Some(doc) = self.active_doc() {
                    let parsed = Markdown::parse(&doc.text);
                    if let Some(link) = parsed.source_link(source_byte(&doc.text, caret)) {
                        self.follow(&link.target, viewport)?;
                    }
                }
            }
            Action::LinkHistory { forward, viewport } => self.walk_link_history(forward, viewport),
            Action::Format { action, selection } => {
                let doc = self.active_doc().ok_or(tr(L::OpenFirst))?;
                self.editor_selection = Some(format::apply(doc, action, selection)?);
            }
            Action::ReplaceSource {
                query,
                replacement,
                options,
                selection,
            } => {
                let doc = self.active_doc().ok_or(tr(L::OpenFirst))?;
                let selected = if let Some([first, last]) = selection {
                    let (_, map) = source_view(&doc.text);
                    let first = map
                        .get(first)
                        .copied()
                        .flatten()
                        .ok_or(tr(L::UnsupportedEdit))?;
                    let last = map
                        .get(last)
                        .copied()
                        .flatten()
                        .ok_or(tr(L::UnsupportedEdit))?;
                    if first > last {
                        return Err(tr(L::UnsupportedEdit).into());
                    }
                    Some(first..last)
                } else {
                    None
                };
                if let Some((caret, count)) =
                    doc.replace_source(&query, &replacement, options, selected)
                {
                    self.jump = Some(caret);
                    self.notice = tr(L::ReplacedCount).replace("{count}", &count.to_string());
                } else {
                    self.notice = tr(L::NoReplaceableResults).into();
                }
            }
            Action::InsertImage { selection } => {
                if let Some(doc) = self.active_doc().filter(|doc| doc.mode == 2) {
                    let document = doc.path().ok_or(tr(L::SaveBeforeRelative))?;
                    let Some(path) = dialog::path(window, Pick::Image, Some(document)) else {
                        return Ok(false);
                    };
                    let parsed = format::projection(doc);
                    let region = selection
                        .map(|part| {
                            part.checked(&parsed)
                                .map(|(index, _)| index)
                                .ok_or(tr(L::ChooseTextBlock))
                        })
                        .transpose()?;
                    let edit =
                        crate::images::insertion(document, &doc.text, &parsed, region, &path)?;
                    doc.patch(edit.range, &edit.text, false);
                    self.editor_selection = Some(format::CaretTarget::Source(edit.caret));
                }
            }
            Action::ReplaceVisual { selection, text } => {
                let doc = self.active_doc().ok_or(tr(L::OpenFirst))?;
                self.editor_selection = Some(format::CaretTarget::Source(
                    format::replace_selection(doc, selection, &text)?,
                ));
            }
            Action::ToggleTask { region } => {
                if let Some(doc) = self.active_doc()
                    && doc.mode == 2
                {
                    let parsed = Markdown::parse(&doc.text);
                    if let Some((range, value)) = parsed
                        .regions
                        .get(region)
                        .and_then(|region| region.list.as_ref())
                        .and_then(|list| list.task_patch(&doc.text))
                    {
                        doc.patch(range, value, false);
                    }
                }
            }
            Action::Review { operation } => {
                let index = self.active.ok_or(tr(L::OpenFirst))?;
                self.notice = self.reviews.apply(&mut self.docs[index], operation)?.into();
            }
            Action::OpenDropped if !self.wait => {
                for path in std::mem::take(&mut self.dropped) {
                    if let Err(error) = self.open(path, 0) {
                        self.notice = format!("{}: {error}", tr(L::OpenError));
                    }
                }
            }
            Action::Theme { theme } => {
                self.preferences.theme = theme;
                self.preferences.write()?;
            }
            Action::Language { english } => {
                i18n::set_language(english);
                self.preferences.english = english;
                self.preferences.write()?;
            }
            Action::Exit => {
                for index in 0..self.docs.len() {
                    if !self.confirms(index, window)? {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    fn follow(&mut self, raw: &str, viewport: Option<LinkViewport>) -> Result<()> {
        let Some(index) = self.active else {
            return Ok(());
        };
        let document = &self.docs[index];
        let from = document.link_position(viewport);
        let anchor = match destination(raw).map_err(|error| match error {
            TargetError::Invalid => tr(L::InvalidLink),
            TargetError::Unsupported => tr(L::LinkUnavailable),
            TargetError::Decode(_) => tr(L::InvalidLinkEscape),
        })? {
            Destination::Anchor(anchor) => anchor,
            Destination::External(url) => {
                open_external(OsStr::new(&url))?;
                return Ok(());
            }
            Destination::Local {
                path: target,
                anchor,
            } => {
                let target = document.resolve_link(&target)?;
                if Some(target.as_path()) != from.path.as_deref() {
                    if self.wait {
                        return Err(tr(L::WaitLink).into());
                    }
                    if !markdown_path(&target) {
                        open_external(target.as_os_str())?;
                        return Ok(());
                    }
                    self.open(target, from.mode & 1)?;
                }
                anchor
            }
        };
        if anchor.is_empty() {
            self.jump = raw.contains('#').then_some(0);
        } else if let Some(doc) = self.active_doc() {
            self.jump = Markdown::parse(&doc.text).anchor_byte(&anchor);
            if self.jump.is_none() {
                self.notice = format!("{}: #{anchor}", tr(L::AnchorNotFound));
            }
        }
        if self.active != Some(index) || self.jump.is_some() {
            self.history.visit(from);
        }
        Ok(())
    }

    fn walk_link_history(&mut self, forward: bool, viewport: Option<LinkViewport>) {
        if !self.history.available(forward) {
            return;
        }
        let from = self
            .active
            .map(|index| self.docs[index].link_position(viewport));
        let mut failure = tr(L::LinkLocationUnavailable).to_owned();
        while let Some(destination) = self.history.pop(forward) {
            if let Some(index) = destination.open_tab(&self.docs) {
                self.active = Some(index);
            } else if let Some(path) = destination.path.filter(|path| path.is_file()) {
                if let Err(error) = self.open(path, destination.mode) {
                    failure = error.to_string();
                    continue;
                }
            } else {
                continue;
            }
            let Some(doc) = self.active_doc() else {
                continue;
            };
            doc.mode = doc.mode & !1 | destination.mode & 1;
            if let Some(from) = from {
                self.history.leave(forward, from);
            }
            self.link_viewport = destination.viewport;
            return;
        }
        self.notice = failure;
    }

    fn next_lint(&mut self, previous: bool, caret: Option<usize>, region: Option<usize>) {
        let Some(index) = self.active else { return };
        let doc = &self.docs[index];
        let parsed = Markdown::parse(&doc.text);
        let issues = match doc.path() {
            Some(path) => pane_md::lint::check_parsed_at(&doc.text, &parsed, path),
            None => pane_md::lint::check_parsed(&doc.text, &parsed),
        };
        let lines = pane_md::markdown::Lines::new(&doc.text);
        let at = caret
            .map(|offset| source_byte(&doc.text, offset))
            .or_else(|| {
                region
                    .and_then(|index| parsed.regions.get(index))
                    .map(|region| region.source.start)
            });
        let locations = || {
            issues
                .iter()
                .filter_map(|issue| Some((lines.offset(issue.line, issue.column)?, issue)))
        };
        let item = if previous {
            locations()
                .rfind(|&(pos, _)| at.is_none_or(|point| pos < point))
                .or_else(|| locations().next_back())
        } else {
            locations()
                .find(|&(pos, _)| at.is_none_or(|point| pos > point))
                .or_else(|| locations().next())
        };
        if let Some((pos, issue)) = item {
            self.jump = Some(pos);
            self.notice = format!(
                "{}:{}  {}",
                issue.line,
                issue.column,
                i18n::lint_label(issue.code)
            );
        } else {
            self.notice = tr(L::LintClean).into();
        }
    }

    fn dark(&self) -> bool {
        match self.preferences.theme {
            Theme::Light => false,
            Theme::Dark => true,
            Theme::System => self.system_dark,
        }
    }

    fn snapshot(&mut self, notify: impl Fn() + Send + 'static) -> Value {
        let tabs: Vec<_> = self.docs.iter().map(|doc| {
            json!({ "id": doc.id, "name": doc.name(), "path": doc.path().map(Path::to_string_lossy), "dirty": doc.dirty() })
        }).collect();
        let active = self.active.map(|index| &self.docs[index]);
        let entries = self
            .folder
            .as_ref()
            .map(|folder| folder.rows(active.and_then(Document::path)))
            .unwrap_or_default();
        let parsed = active.map(format::projection);
        self.diagrams.prepare(
            parsed
                .as_ref()
                .filter(|_| active.is_some_and(|doc| doc.mode.is_multiple_of(2))),
            self.preferences.appearance.diagram_dark(self.dark()),
            notify,
        );
        let diagrams = self.diagrams.snapshot();
        let document = active.zip(parsed).map(|(doc, parsed)| {
            let (display, map) = source_view(&doc.text);
            let points: Vec<_> = map.into_iter().enumerate().filter_map(|(unit, byte)| Some((byte?, unit))).collect();
            let offset = |byte| points.get(points.partition_point(|&(at, _)| at < byte))
                .map_or(display.len(), |&(_, unit)| unit);
            let headings: Vec<_> = parsed.regions.iter().enumerate().filter(|(_, region)| region.level > 0)
                .map(|(region, data)| json!({ "region": region, "level": data.level, "text": data.text, "offset": offset(data.source.start) }))
                .collect();
            let reviews: Vec<_> = parsed.records.iter().map(|record| {
                let location = match &record.binding {
                    Binding::Attached { region, bytes, .. } => json!({
                        "region": region, "offset": offset(bytes.start), "endOffset": offset(bytes.end)
                    }),
                    _ => Value::Null,
                };
                json!({
                    "id": record.review.id, "text": record.review.text, "quote": record.review.quote,
                    "status": record.review.status, "location": location, "date": record.review.updated_at,
                    "ambiguous": matches!(record.binding, Binding::Ambiguous),
                })
            }).collect();
            let source_records: Vec<_> = if doc.mode % 2 == 1 {
                parsed.records.iter().map(|record| {
                    [offset(record.source.start), offset(record.source.end)]
                }).collect()
            } else {
                Vec::new()
            };
            let jump = self.jump.map(|byte| {
                let region = parsed.regions.iter().enumerate().find(|(_, item)| item.source.contains(&byte))
                    .map(|(index, _)| index);
                json!({"offset": offset(byte), "endOffset": self.jump_end.map(offset), "region": region})
            });
            json!({
                "source": if doc.mode % 2 == 1 { String::from_utf16_lossy(&display).replace('\r', "\n") } else { String::new() },
                "html": if doc.mode.is_multiple_of(2) {
                    preview::html(&parsed, doc.path(), &doc.text, doc.mode >= 2 && !self.reviews.is_drafting(), doc.show_reviews)
                } else { String::new() },
                "mode": doc.mode,
                "showReviews": doc.show_reviews,
                "diagrams": diagrams,
                "headings": headings,
                "sourceRecords": source_records,
                "reviews": reviews,
                "reviewEditor": self.reviews.snapshot(&parsed),
                "jump": jump,
                "editorSelection": self.editor_selection.as_ref().and_then(|target| target.locate(&parsed)),
            })
        });
        json!({
            "labels": labels(),
            "english": self.preferences.english,
            "theme": self.preferences.theme,
            "appearance": self.preferences.appearance.snapshot(),
            "tabs": tabs,
            "active": self.active,
            "folder": self.folder.as_ref().map(|folder| folder.path.to_string_lossy()),
            "folderHasParent": self.folder.as_ref().is_some_and(|folder| folder.path.parent().is_some()),
            "entries": entries,
            "document": document,
            "notice": self.notice,
            "wait": self.wait,
            "canGoBack": self.history.available(false),
            "canGoForward": self.history.available(true),
            "macos": cfg!(target_os = "macos"),
            "linkViewport": self.link_viewport,
        })
    }
}

fn source_byte(text: &str, caret: usize) -> usize {
    source_view(text)
        .1
        .into_iter()
        .take(caret.saturating_add(1))
        .flatten()
        .next_back()
        .unwrap_or(0)
}

fn open_external(target: &OsStr) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    Command::new(opener)
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

fn labels() -> Value {
    Value::Object(
        [
            ("file", L::MenuFile),
            ("quit", L::QuitMenu),
            ("edit", L::MenuEdit),
            ("format", L::MenuFormat),
            ("formatInVisualEdit", L::FormatInVisualEdit),
            ("openTab", L::OpenTab),
            ("richFamily", L::RichFamily),
            ("sourceFamily", L::SourceFamily),
            ("representation", L::SwitchFamily),
            ("documentMode", L::DocumentMode),
            ("sourceEditor", L::SourceEditor),
            ("zoomIn", L::ZoomInButton),
            ("zoomOut", L::ZoomOutButton),
            ("zoomReset", L::ZoomResetButton),
            ("formatting", L::Formatting),
            ("paragraphAndLists", L::ParagraphAndLists),
            ("tableCommands", L::TableCommands),
            ("bold", L::Bold),
            ("italic", L::Italic),
            ("strike", L::Strike),
            ("code", L::Code),
            ("paragraph", L::Paragraph),
            ("heading1", L::Heading1),
            ("heading2", L::Heading2),
            ("heading3", L::Heading3),
            ("heading4", L::Heading4),
            ("heading5", L::Heading5),
            ("heading6", L::Heading6),
            ("bullet", L::Bullet),
            ("numbered", L::NumberedItem),
            ("task", L::MakeTask),
            ("insertTable", L::InsertTable),
            ("insertRow", L::InsertRow),
            ("deleteRow", L::DeleteRow),
            ("insertColumn", L::InsertColumn),
            ("deleteColumn", L::DeleteColumn),
            ("insertLink", L::InsertLink),
            ("insertImage", L::InsertImage),
            ("linkAddress", L::LinkAddress),
            ("editLink", L::EditLink),
            ("removeLink", L::RemoveLink),
            ("chooseTextBlock", L::ChooseTextBlock),
            ("selectFormatting", L::SelectFormatting),
            ("view", L::MenuView),
            ("settings", L::MenuSettings),
            ("open", L::OpenFileMenu),
            ("newNote", L::NewNote),
            ("folder", L::OpenDirectoryMenu),
            ("refreshFiles", L::RefreshFiles),
            ("save", L::SaveMenu),
            ("saveAs", L::SaveAsMenu),
            ("close", L::CloseTab),
            ("undo", L::UndoMenu),
            ("redo", L::RedoMenu),
            ("find", L::FindMenu),
            ("findHeading", L::FindHeading),
            ("findHeadingMenu", L::FindHeadingMenu),
            ("showSource", L::ShowSource),
            ("showReplace", L::ReplaceMenu),
            ("replacement", L::ReplacementText),
            ("replaceOne", L::ReplaceOne),
            ("replaceAll", L::ReplaceAll),
            ("matchCase", L::MatchCase),
            ("wholeWord", L::WholeWord),
            ("searchCount", L::SearchCount),
            ("findNext", L::FindNext),
            ("findPrevious", L::FindPrevious),
            ("lint", L::LintNext),
            ("back", L::BackLink),
            ("forward", L::ForwardLink),
            ("read", L::View),
            ("comments", L::Review),
            ("editing", L::Edit),
            ("outline", L::Outline),
            ("files", L::Files),
            ("reviews", L::Comments),
            ("light", L::LightTheme),
            ("dark", L::DarkTheme),
            ("system", L::SystemTheme),
            ("appearance", L::Appearance),
            ("richFont", L::RichFont),
            ("sourceFont", L::SourceFont),
            ("fontSize", L::FontSize),
            ("defaultFont", L::DefaultFont),
            ("themeColors", L::ThemeColors),
            ("paperColor", L::PaperColor),
            ("textColor", L::TextColor),
            ("linkColor", L::LinkColor),
            ("resetColors", L::ResetColors),
            ("resetAppearance", L::ResetAppearance),
            ("contrastHint", L::AppearanceContrastHint),
            ("ru", L::Russian),
            ("en", L::English),
            ("up", L::UpFolder),
            ("empty", L::EmptyEditor),
            ("noMatch", L::NoResults),
            ("emptyReview", L::NoComments),
            ("showOutline", L::ShowOutline),
            ("showComments", L::ShowComments),
            ("hideOutline", L::HideOutline),
            ("hideFiles", L::HideFiles),
            ("hideComments", L::HideComments),
            ("resolved", L::ReviewResolved),
            ("unattached", L::ReviewUnattached),
            ("ambiguous", L::ReviewAmbiguous),
            ("all", L::All),
            ("openComments", L::OpenComments),
            ("addReview", L::AddComment),
            ("editReview", L::ReviewEdit),
            ("resolveReview", L::ResolveComment),
            ("reopenReview", L::ReopenComment),
            ("reattachReview", L::Reattach),
            ("deleteReview", L::Delete),
            ("confirmDelete", L::ConfirmDelete),
            ("apply", L::Apply),
            ("saveComment", L::SaveComment),
            ("cancel", L::Cancel),
            ("newReview", L::NewReview),
            ("comment", L::Comment),
            ("more", L::More),
            ("selectForReview", L::SelectForReviewPortable),
        ]
        .into_iter()
        .map(|(name, key)| {
            let translated = tr(key).split('\t').next().unwrap_or("").replace('&', "");
            (name.to_owned(), Value::String(translated))
        })
        .collect(),
    )
}

enum HostEvent {
    Message(Message),
    #[cfg(target_os = "linux")]
    PreviousTab,
    Drop(Vec<PathBuf>),
    DiagramReady,
    AppearanceFonts(Vec<String>),
}

fn asset(request: Request<Vec<u8>>) -> Response<std::borrow::Cow<'static, [u8]>> {
    if request.uri().path() == "/theme.css" {
        return Response::builder()
            .header("Content-Type", "text/css; charset=utf-8")
            .body(appearance::theme_css().into_bytes().into())
            .unwrap();
    }
    let (content, bytes): (&str, &[u8]) = match request.uri().path() {
        "/" => ("text/html; charset=utf-8", include_bytes!("app.html")),
        "/app.css" => ("text/css; charset=utf-8", include_bytes!("app.css")),
        "/components.js" => (
            "application/javascript; charset=utf-8",
            include_bytes!("components.js"),
        ),
        "/source-pane.js" => (
            "application/javascript; charset=utf-8",
            include_bytes!("source-pane.js"),
        ),
        "/appearance.js" => (
            "application/javascript; charset=utf-8",
            include_bytes!("appearance.js"),
        ),
        "/app.js" => (
            "application/javascript; charset=utf-8",
            include_bytes!("app.js"),
        ),
        "/format.js" => (
            "application/javascript; charset=utf-8",
            include_bytes!("format.js"),
        ),
        "/review-pane.js" => (
            "application/javascript; charset=utf-8",
            include_bytes!("review-pane.js"),
        ),
        "/icon.png" => ("image/png", ICON),
        _ => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body((&[][..]).into())
                .unwrap();
        }
    };
    Response::builder()
        .header("Content-Type", content)
        .header("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; img-src data: 'self' https: http:; base-uri 'none'; form-action 'none'")
        .body(bytes.into()).unwrap()
}

fn own_page(uri: &Uri) -> bool {
    uri.scheme_str() == Some("panemd")
        && uri.authority().is_some_and(|host| host.as_str() == "app")
        && uri.path() == "/"
}

fn window_icon() -> Option<Icon> {
    let mut decoder = png::Decoder::new(io::Cursor::new(ICON));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut bytes = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut bytes).ok()?;
    (info.color_type == png::ColorType::Rgba)
        .then(|| Icon::from_rgba(bytes, info.width, info.height).ok())
        .flatten()
}

pub fn spawn_gui(path: Option<&Path>, stderr: Stdio) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    require_display()?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--gui")
        .args(path)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);

    // A long-lived MCP server must reap editors that close before it exits.
    let (launched, reply) = std::sync::mpsc::sync_channel(0);
    std::thread::Builder::new()
        .name("editor-process".into())
        .spawn(move || match command.spawn() {
            Ok(mut child) => {
                let _ = launched.send(Ok(()));
                let _ = child.wait();
            }
            Err(error) => {
                let _ = launched.send(Err(error));
            }
        })?;
    reply.recv().map_err(io::Error::other)?
}

#[cfg(target_os = "linux")]
fn require_display() -> io::Result<()> {
    if ["DISPLAY", "WAYLAND_DISPLAY"]
        .into_iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            tr(L::NoDisplay),
        ))
    }
}

pub fn run_gui(path: Option<PathBuf>, wait: bool) -> Result<()> {
    let preferences = Preferences::read();
    i18n::set_language(preferences.english);
    #[cfg(target_os = "linux")]
    require_display()?;
    let mut state = Workspace {
        preferences,
        wait,
        ..Workspace::default()
    };
    if let Some(path) = path {
        state.open(path, if wait { 3 } else { 0 })?;
    } else if wait {
        return Err(tr(L::WaitNeedsFile).into());
    }
    let events = EventLoopBuilder::<HostEvent>::with_user_event().build();
    let window = WindowBuilder::new()
        .with_title("pane-md")
        .with_inner_size(LogicalSize::new(1160, 780))
        .with_min_inner_size(LogicalSize::new(500, 370))
        .with_window_icon(window_icon())
        .build(&events)?;
    let ipc = events.create_proxy();
    let drop_events = events.create_proxy();
    let font_events = events.create_proxy();
    let diagram_events = events.create_proxy();
    let notify = move || {
        let _ = diagram_events.send_event(HostEvent::DiagramReady);
    };
    let assets = state.diagrams.assets.clone();
    let web = WebViewBuilder::new()
        .with_url(ORIGIN)
        .with_custom_protocol("panemd".into(), move |_, request| {
            assets.response(&request).unwrap_or_else(|| asset(request))
        })
        .with_ipc_handler(move |request| {
            if own_page(request.uri())
                && let Ok(message) = serde_json::from_str(request.body())
            {
                let _ = ipc.send_event(HostEvent::Message(message));
            }
        })
        .with_navigation_handler(|address| address.parse::<Uri>().is_ok_and(|uri| own_page(&uri)))
        .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
        .with_drag_drop_handler(move |event| {
            if !wait && let wry::DragDropEvent::Drop { paths, .. } = event {
                let _ = drop_events.send_event(HostEvent::Drop(paths));
            }
            true
        });
    #[cfg(target_os = "macos")]
    let web = web.build(&window)?;
    #[cfg(target_os = "linux")]
    let web = {
        use gtk::{gdk, glib::Propagation, prelude::WidgetExt};
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let shortcuts = events.create_proxy();
        window
            .gtk_window()
            .connect_key_press_event(move |_, event| {
                use gdk::{ModifierType as M, keys::constants};
                let modifiers = event.state();
                if modifiers.contains(M::CONTROL_MASK | M::SHIFT_MASK)
                    && !modifiers.intersects(M::MOD1_MASK | M::SUPER_MASK | M::META_MASK)
                    && matches!(event.keyval(), constants::Tab | constants::ISO_Left_Tab)
                {
                    let _ = shortcuts.send_event(HostEvent::PreviousTab);
                    Propagation::Stop
                } else {
                    Propagation::Proceed
                }
            });
        web.build_gtk(
            window
                .default_vbox()
                .ok_or("GTK window has no content container")?,
        )?
    };
    let mut ready = false;
    let mut webview = Some(web);
    events.run(move |event, _, control| {
        if matches!(event, Event::LoopDestroyed) {
            drop(webview.take());
            return;
        }
        let Some(web) = webview.as_ref() else { return };
        *control = ControlFlow::Wait;
        let result = match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if ready {
                    let _ = web.evaluate_script("window.requestClose()");
                } else {
                    drop(webview.take());
                    *control = ControlFlow::Exit;
                }
                return;
            }
            Event::UserEvent(HostEvent::Message(message)) => {
                match &message.action {
                    Action::SystemAppearance { dark } => {
                        state.system_dark = *dark;
                        if matches!(state.preferences.theme, Theme::System) {
                            state.diagrams.set_theme(
                                state.preferences.appearance.diagram_dark(*dark),
                                notify.clone(),
                            );
                            let _ = web.evaluate_script(&format!(
                                "window.updateDiagrams({})",
                                json!(state.diagrams.snapshot())
                            ));
                        }
                        return;
                    }
                    Action::AppearanceFonts => {
                        let font_events = font_events.clone();
                        let spawned = std::thread::Builder::new()
                            .name("editor-fonts".into())
                            .spawn(move || {
                                let _ = font_events.send_event(HostEvent::AppearanceFonts(
                                    appearance::installed_fonts(),
                                ));
                            });
                        if spawned.is_err() {
                            let _ = web.evaluate_script("window.setEditorFonts([])");
                        }
                        return;
                    }
                    Action::SetEditorAppearance { appearance } => {
                        match state.preferences.set_appearance(appearance.clone()) {
                            Ok(()) => {
                                state.diagrams.set_theme(
                                    state.preferences.appearance.diagram_dark(state.dark()),
                                    notify.clone(),
                                );
                                let _ = web.evaluate_script(&format!(
                                    "window.setEditorAppearance({}); window.updateDiagrams({});",
                                    state.preferences.appearance.snapshot(),
                                    json!(state.diagrams.snapshot())
                                ));
                            }
                            Err(error) => {
                                let _ = web.evaluate_script(&format!(
                                    "window.editorAppearanceError({})",
                                    json!(format!("{}. {error}", tr(L::AppearanceError)))
                                ));
                            }
                        }
                        return;
                    }
                    _ => {}
                }
                ready = true;
                let operation = match &message.action {
                    Action::Review { operation } => Some(operation),
                    _ => None,
                };
                if !state.reviews.allows(operation) {
                    let _ = web.evaluate_script(&format!(
                        "window.showError({})",
                        json!(tr(L::FinishDraft))
                    ));
                    return;
                }
                if !state.reviews.is_drafting()
                    && let Some(doc) = state.active_doc()
                    && let Err(error) = Workspace::apply_edits(doc, message.source, message.edits)
                {
                    let _ = web.evaluate_script(&format!(
                        "window.showError({})",
                        json!(error.to_string())
                    ));
                    return;
                }
                let previous = state.active.map(|index| state.docs[index].id);
                let result = state.act(message.action, &window);
                if previous != state.active.map(|index| state.docs[index].id) {
                    state.reviews = reviews::Editor::default();
                }
                result
            }
            Event::UserEvent(HostEvent::AppearanceFonts(fonts)) => {
                let _ = web.evaluate_script(&format!("window.setEditorFonts({})", json!(fonts)));
                return;
            }
            #[cfg(target_os = "linux")]
            Event::UserEvent(HostEvent::PreviousTab) => {
                let _ = web.evaluate_script("window.selectPreviousTab()");
                return;
            }
            Event::UserEvent(HostEvent::DiagramReady) => {
                if state.diagrams.receive() {
                    let _ = web.evaluate_script(&format!(
                        "window.updateDiagrams({})",
                        json!(state.diagrams.snapshot())
                    ));
                }
                return;
            }
            Event::UserEvent(HostEvent::Drop(paths)) => {
                state.dropped.extend(paths);
                let _ = web.evaluate_script("window.acceptDrop()");
                return;
            }
            #[cfg(target_os = "macos")]
            Event::Opened { urls } => {
                if !state.wait {
                    state
                        .dropped
                        .extend(urls.into_iter().filter_map(|url| url.to_file_path().ok()));
                    if ready && !state.reviews.is_drafting() && !state.dropped.is_empty() {
                        let _ = web.evaluate_script("window.acceptDrop()");
                    }
                }
                return;
            }
            _ => return,
        };
        match result {
            Ok(true) => {
                // Tao exits the process directly, so the WebView must be destroyed here.
                drop(webview.take());
                *control = ControlFlow::Exit;
            }
            Ok(false) => {
                let title = state
                    .active_doc()
                    .map(|doc| format!("{} — pane-md", doc.name()));
                window.set_title(title.as_deref().unwrap_or("pane-md"));
                let _ = web.evaluate_script(&format!(
                    "window.render({})",
                    state.snapshot(notify.clone())
                ));
                if !state.reviews.is_drafting() && !state.dropped.is_empty() {
                    let _ = web.evaluate_script("window.acceptDrop()");
                }
            }
            Err(error) => {
                state.notice = error.to_string();
                let _ = web.evaluate_script(&format!(
                    "window.render({})",
                    state.snapshot(notify.clone())
                ));
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn link_history_skips_removed_tabs_and_keeps_the_surviving_locations() -> Result<()> {
        let project = tempfile::tempdir()?;
        let root = project.path().canonicalize()?;
        let first = root.join("First.md");
        let removed = root.join("Removed.md");
        let last = root.join("Last.md");
        fs::write(&first, "# First\n")?;
        fs::write(&removed, "# Removed\n")?;
        fs::write(&last, "# Last\n")?;
        let mut workspace = Workspace::default();
        workspace.open(first.clone(), 1)?;
        let view = |selection| {
            Some(LinkViewport {
                scroll_top: 16.0,
                scroll_left: 3.0,
                selection,
                visual_selection: None,
                jump_region: None,
            })
        };
        workspace.follow("Removed.md", view([1, 4]))?;
        workspace.follow("Last.md", view([2, 3]))?;
        workspace.docs.remove(1);
        workspace.active = Some(1);
        fs::remove_file(removed)?;

        workspace.walk_link_history(false, view([1, 3]));
        assert_eq!(
            workspace.active_doc().unwrap().path(),
            Some(first.as_path())
        );
        assert_eq!(workspace.link_viewport.as_ref().unwrap().selection, [1, 4]);
        workspace.walk_link_history(true, view([2, 5]));
        assert_eq!(workspace.active_doc().unwrap().path(), Some(last.as_path()));
        assert_eq!(workspace.link_viewport.as_ref().unwrap().selection, [1, 3]);
        workspace.walk_link_history(false, view([1, 3]));
        workspace.follow("#missing", None)?;
        assert!(workspace.history.available(true));
        workspace.follow("#first", None)?;
        assert!(!workspace.history.available(true));
        Ok(())
    }

    #[test]
    fn workspace_source_component_receives_utf16_comment_offsets() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("README.md");
        let record = r#"<!-- pane-md:review {"v":1,"id":"sdk","quote":"SDK","anchor":{"heading":"План","block":"paragraph","prefix":"😀 ","suffix":"."},"text":"Проверить SDK.","status":"open"} -->"#;
        fs::write(
            &path,
            format!("\u{feff}# План\r\n\r\n😀 SDK.\r\n\r\n{record}\r\n"),
        )?;
        let mut state = Workspace::default();
        state.open(path, 1)?;
        let state = state.snapshot(|| {});
        let doc = &state["document"];
        let units: Vec<_> = doc["source"].as_str().unwrap().encode_utf16().collect();
        let slice = |start: &Value, end: &Value| {
            String::from_utf16_lossy(
                &units[start.as_u64().unwrap() as usize..end.as_u64().unwrap() as usize],
            )
        };
        assert_eq!(doc["headings"][0]["offset"], 0);
        let location = &doc["reviews"][0]["location"];
        assert_eq!(slice(&location["offset"], &location["endOffset"]), "SDK");
        let span = &doc["sourceRecords"][0];
        assert_eq!(slice(&span[0], &span[1]).trim(), record);
        Ok(())
    }

    #[test]
    fn appearance_preferences_round_trip_and_load_older_settings() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("pane-md/settings.json");
        let legacy = folder.path().join("notepad-md/settings.json");
        fs::create_dir_all(legacy.parent().unwrap())?;
        fs::write(&legacy, r#"{"english":true,"theme":"dark"}"#)?;
        assert!(Preferences::load_with_legacy(&path).unwrap().english);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, r#"{"english":true,"theme":"dark"}"#)?;
        let mut preferences = Preferences::load(&path).unwrap();
        assert!(preferences.english && matches!(preferences.theme, Theme::Dark));
        assert_eq!(preferences.appearance, appearance::Appearance::default());
        preferences.appearance = serde_json::from_str(
            r#"{"rich":{"family":"DejaVu Serif","half_points":38},"dark":{"paper":2631720}}"#,
        )?;
        preferences.write_to(&path)?;
        let restored = Preferences::load(&path).unwrap();
        assert_eq!(preferences.appearance, restored.appearance);
        assert!(restored.english && matches!(restored.theme, Theme::Dark));
        Ok(())
    }

    #[test]
    fn source_mode_saves_line_endings_and_preserves_external_changes() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("notes.md");
        let original = "\u{feff}# План\r\n\r\nЧерновик 😀\r\n";
        fs::write(&path, original)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;
        let mut doc = Document::open(path.clone(), 3)?;
        Workspace::apply_edits(&mut doc, Some("# План\n\nГотово 😀\n".into()), Vec::new())?;
        doc.save()?;
        assert_eq!(
            fs::read_to_string(&path)?,
            original.replace("Черновик", "Готово")
        );
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o640);
        Workspace::apply_edits(
            &mut doc,
            Some("# План\n\nЛокальная правка 😀\n".into()),
            Vec::new(),
        )?;
        fs::write(&path, "# External\n")?;
        assert!(doc.save().is_err());
        assert_eq!(fs::read_to_string(path)?, "# External\n");
        assert!(doc.dirty());
        assert!(doc.text.contains("Локальная правка"));
        Ok(())
    }

    #[test]
    fn pending_edits_to_implicit_cells_keep_their_columns() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("cells.md");
        let original = "| A | B | C |\r\n| --- | --- | --- |\r\n| x |\r\n|   |";
        fs::write(&path, original)?;
        let mut doc = Document::open(path, 2)?;
        let edits = [(7, "😀"), (5, "Cell | C"), (6, "first"), (4, "Cell B")]
            .into_iter()
            .map(|(region, text)| RichEdit {
                region,
                start: 0,
                end: 0,
                text: text.into(),
            })
            .collect();
        Workspace::apply_edits(&mut doc, None, edits)?;
        assert_eq!(
            doc.text,
            "| A | B | C |\r\n| --- | --- | --- |\r\n| x | Cell B | Cell \\| C |\r\n|   first| 😀 |"
        );
        let parsed = Markdown::parse(&doc.text);
        assert_eq!(
            parsed
                .regions
                .iter()
                .skip(3)
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            ["x", "Cell B", "Cell | C", "first", "😀", ""]
        );
        Ok(())
    }

    #[test]
    fn visual_edit_updates_only_its_markdown_span() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("notes.md");
        let original = "# Heading 😀\r\n\r\nA **strong\u{a0}word** and *italic*.\r\n";
        fs::write(&path, original)?;
        let mut doc = Document::open(path, 2)?;
        let parsed = Markdown::parse(&doc.text);
        let (region, selected) = parsed
            .regions
            .iter()
            .enumerate()
            .find(|(_, region)| region.text.starts_with("A strong"))
            .ok_or("missing region")?;
        let edit = RichEdit {
            region,
            start: 2,
            end: 2 + "strong\u{a0}word".encode_utf16().count(),
            text: "strong\u{a0}✅".into(),
        };
        Workspace::apply_edits(&mut doc, None, vec![edit])?;
        assert_eq!(doc.text, original.replace("word", "✅"));
        assert!(doc.undo(false).is_some());
        assert_eq!(doc.text, original);
        assert!(!selected.text.is_empty());
        Ok(())
    }

    #[test]
    fn local_and_embedded_svg_are_rendered_as_png() -> Result<()> {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("notes.md");
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><rect width="80" height="30" fill="#215dab"/><image width="80" height="30" href="https://invalid.example/source.png"/></svg>"##;
        fs::write(folder.path().join("figure.svg"), svg)?;
        let source = format!(
            "[![local](figure.svg)](#figure)\n\n![inline](data:image/svg+xml;base64,{})",
            STANDARD.encode(svg)
        );
        let rendered = preview::html(
            &Markdown::parse(&source),
            Some(&path),
            &source,
            false,
            false,
        );
        assert_eq!(
            rendered
                .matches("<img src=\"data:image/png;base64,")
                .count(),
            2
        );
        assert!(!rendered.contains("https://invalid.example"));
        assert!(!rendered.contains("<svg"));
        assert!(rendered.contains("<a href=\"#\" data-link=\"#figure\"><img "));
        Ok(())
    }

    #[test]
    fn preview_renders_local_images_and_escapes_markdown_html() -> Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("notes.md");
        fs::write(folder.path().join("picture.png"), ICON)?;
        let text = "# Notes\n\n![icon](picture.png)\n\n<img src=\"https://invalid.example/asset\" onload=\"noop\">\n";
        let html = preview::html(&Markdown::parse(text), Some(&path), text, false, false);
        assert!(html.contains("<img src=\"data:image/png;base64,"));
        assert!(html.contains("&lt;img"));
        assert!(!html.contains("<img src=\"https://"));
        assert!(own_page(&ORIGIN.parse()?));
        assert!(!own_page(&"panemd://app.example/".parse()?));
        assert_eq!(
            asset(
                Request::builder()
                    .uri("panemd://app/private")
                    .body(Vec::new())?
            )
            .status(),
            StatusCode::NOT_FOUND,
        );
        Ok(())
    }

    #[test]
    fn remote_markdown_image_waits_for_a_click_and_escapes_its_url() {
        let source = "[![chart](https://images.example/chart.png?x=1&y=2)](#chart)";
        let html = preview::html(&Markdown::parse(source), None, source, false, false);
        assert!(
            html.contains("data-remote-image=\"https://images.example/chart.png?x=1&amp;y=2\"")
        );
        assert!(
            html.contains(
                "data-load-remote-image=\"https://images.example/chart.png?x=1&amp;y=2\""
            )
        );
        assert!(html.contains("data-link=\"#chart\""));
        assert!(!html.contains("src=\"https://"));
    }
}
