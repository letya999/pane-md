mod file;
mod mcp;
mod rich;
mod ui;

use std::{
    ffi::OsString,
    io,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const USAGE: &str = "notepad-md [path]\nnotepad-md --source --wait path\nnotepad-md --mcp";

pub fn run() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next();
    match first.as_ref().and_then(|arg| arg.to_str()) {
        Some("--mcp") if arguments.len() == 0 => mcp::run(),
        Some("--help" | "-h") if arguments.len() == 0 => {
            println!("{USAGE}");
            Ok(())
        }
        Some("--gui") if arguments.len() <= 1 => {
            ui::run(arguments.next().map(PathBuf::from), false)
        }
        Some("--source")
            if arguments.len() == 2 && arguments.next() == Some(OsString::from("--wait")) =>
        {
            ui::run(arguments.next().map(PathBuf::from), true)
        }
        Some(flag) if flag.starts_with('-') => Err(USAGE.into()),
        _ if arguments.len() == 0 => {
            let path = first
                .map(PathBuf::from)
                .map(|p| std::path::absolute(p))
                .transpose()?;
            if let Some(path) = &path {
                let metadata = std::fs::metadata(path)?;
                if !metadata.is_file() && !metadata.is_dir() {
                    return Err("ожидался файл или папка".into());
                }
            }
            Ok(spawn_gui(path.as_deref())?)
        }
        _ => Err(USAGE.into()),
    }
}

fn spawn_gui(path: Option<&Path>) -> io::Result<()> {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--gui")
        .args(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(DETACHED_PROCESS)
        .spawn()?;
    Ok(())
}
