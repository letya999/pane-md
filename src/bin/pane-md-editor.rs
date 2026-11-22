#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    env,
    process::{Command, ExitCode, Stdio},
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(first) = args.next() else {
        eprintln!("pane-md-editor: expected one file path");
        return ExitCode::FAILURE;
    };
    let open_normally = first == "--open";
    let path = if open_normally {
        args.next()
    } else {
        Some(first)
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
    if !open_normally {
        command.arg("--source").arg("--wait");
    }
    if let Some(path) = path {
        command.arg(path);
    }
    command.stdin(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x0000_0008); // DETACHED_PROCESS: no terminal window.
    if open_normally {
        match command.stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
            Ok(_) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        }
    } else {
        match command.status() {
            Ok(status) if status.success() => ExitCode::SUCCESS,
            Ok(_) | Err(_) => ExitCode::FAILURE,
        }
    }
}
