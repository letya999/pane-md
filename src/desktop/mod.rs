use crate::{document as file, i18n};
mod rich;
mod settings;
mod theme;
mod ui;

use crate::Result;
use std::{
    io,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub fn run_gui(path: Option<PathBuf>, wait: bool) -> Result<()> {
    i18n::set_language(settings::read_preference("Language") == Some(1));
    ui::run(path, wait)
}

pub fn spawn_gui(path: Option<&Path>, stderr: Stdio) -> io::Result<()> {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--gui")
        .args(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .creation_flags(DETACHED_PROCESS)
        .spawn()?;
    Ok(())
}
