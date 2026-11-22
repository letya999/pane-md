use std::path::{Path, PathBuf};

use tao::window::Window;

use crate::i18n::{Label as L, tr};

#[derive(Clone, Copy)]
pub enum Pick<'a> {
    File,
    Folder,
    Image,
    SaveAs(&'a str),
}

impl Pick<'_> {
    fn label(self) -> L {
        match self {
            Self::File => L::OpenDocument,
            Self::Folder => L::OpenFolder,
            Self::Image => L::InsertImage,
            Self::SaveAs(_) => L::SaveAsDialog,
        }
    }
}

#[derive(PartialEq, Eq)]
pub enum SaveChoice {
    Save,
    Discard,
    Cancel,
}

#[cfg(target_os = "linux")]
pub fn path(window: &Window, pick: Pick<'_>, previous: Option<&Path>) -> Option<PathBuf> {
    use gtk::{FileChooserAction, FileChooserNative, ResponseType, prelude::*};
    use tao::platform::unix::WindowExtUnix;

    let parent = window.gtk_window();
    let dialog = FileChooserNative::new(
        Some(tr(pick.label())),
        Some(parent),
        match pick {
            Pick::File | Pick::Image => FileChooserAction::Open,
            Pick::Folder => FileChooserAction::SelectFolder,
            Pick::SaveAs(_) => FileChooserAction::Save,
        },
        Some(tr(pick.label())),
        Some(tr(L::Cancel)),
    );
    if matches!(pick, Pick::Image) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(tr(L::ImageFiles)));
        for ext in crate::images::EXTENSIONS {
            filter.add_pattern(&format!("*.{ext}"));
            filter.add_pattern(&format!("*.{}", ext.to_ascii_uppercase()));
        }
        dialog.add_filter(filter);
    }
    if let Some(previous) = previous {
        if previous.is_dir() {
            dialog.set_current_folder(previous);
        } else if let Some(folder) = previous.parent() {
            dialog.set_current_folder(folder);
        }
    }
    if let Pick::SaveAs(name) = pick {
        dialog.set_current_name(name);
    }
    (dialog.run() == ResponseType::Accept)
        .then(|| dialog.filename())
        .flatten()
}

#[cfg(target_os = "linux")]
pub fn confirm(window: &Window, message: &str) -> SaveChoice {
    use gtk::{ButtonsType, DialogFlags, MessageDialog, MessageType, ResponseType, prelude::*};
    use tao::platform::unix::WindowExtUnix;

    let dialog = MessageDialog::new(
        Some(window.gtk_window()),
        DialogFlags::MODAL,
        MessageType::Warning,
        ButtonsType::None,
        message,
    );
    dialog.add_button(tr(L::Cancel), ResponseType::Cancel);
    dialog.add_button(tr(L::DiscardChanges), ResponseType::No);
    dialog.add_button(tr(L::SaveComment), ResponseType::Yes);
    dialog.set_default_response(ResponseType::Cancel);
    let choice = match dialog.run() {
        ResponseType::Yes => SaveChoice::Save,
        ResponseType::No => SaveChoice::Discard,
        _ => SaveChoice::Cancel,
    };
    // GtkDialog::run hides neither a responding dialog nor its window.
    unsafe { dialog.destroy() };
    choice
}

#[cfg(target_os = "macos")]
pub fn path(window: &Window, pick: Pick<'_>, previous: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title(tr(pick.label()))
        .set_parent(window);
    if matches!(pick, Pick::Image) {
        dialog = dialog.add_filter(tr(L::ImageFiles), crate::images::EXTENSIONS);
    }
    if let Some(previous) = previous {
        if previous.is_dir() {
            dialog = dialog.set_directory(previous);
        } else if let Some(folder) = previous.parent() {
            dialog = dialog.set_directory(folder);
        }
    }
    match pick {
        Pick::File | Pick::Image => dialog.pick_file(),
        Pick::Folder => dialog.pick_folder(),
        Pick::SaveAs(name) => dialog.set_file_name(name).save_file(),
    }
}

#[cfg(target_os = "macos")]
pub fn confirm(window: &Window, message: &str) -> SaveChoice {
    use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

    let save = tr(L::SaveComment);
    let discard = tr(L::DiscardChanges);
    match MessageDialog::new()
        .set_parent(window)
        .set_level(MessageLevel::Warning)
        .set_title("pane-md")
        .set_description(message)
        .set_buttons(MessageButtons::YesNoCancelCustom(
            save.into(),
            discard.into(),
            tr(L::Cancel).into(),
        ))
        .show()
    {
        MessageDialogResult::Yes => SaveChoice::Save,
        MessageDialogResult::No => SaveChoice::Discard,
        MessageDialogResult::Custom(label) if label == save => SaveChoice::Save,
        MessageDialogResult::Custom(label) if label == discard => SaveChoice::Discard,
        _ => SaveChoice::Cancel,
    }
}
