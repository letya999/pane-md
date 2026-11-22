#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    env,
    process::{Command, ExitCode, Stdio},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("pane-md-editor: expected one file path");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("pane-md-editor: expected one file path");
        return ExitCode::FAILURE;
    }
    let Ok(exe) = env::current_exe() else {
        eprintln!("pane-md-editor: cannot locate pane-md");
        return ExitCode::FAILURE;
    };
    let mut command = Command::new(exe.with_file_name(if cfg!(windows) {
        "pane-md.exe"
    } else {
        "pane-md"
    }));
    command
        .arg("--source")
        .arg("--wait")
        .arg(path)
        .stdin(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x0000_0008); // DETACHED_PROCESS: no terminal window.
    match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) | Err(_) => ExitCode::FAILURE,
    }
}
