use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
};

use pane_md::{lint, markdown::Markdown, text_file::decode};
use serde_json::json;

use crate::Result;

pub const HAS_DESKTOP_EDITOR: bool = cfg!(any(
    windows,
    all(
        feature = "desktop-gui",
        any(target_os = "linux", target_os = "macos")
    )
));

const USAGE: &str = "pane-md [PATH]
pane-md --source --wait PATH
pane-md --mcp
pane-md --list-comments [--] PATH
pane-md --lint [--json] [--] PATH...
pane-md --version";

pub fn run(args: impl IntoIterator<Item = OsString>) -> Result<ExitCode> {
    let mut args = args.into_iter();
    let first = args.next();
    let mut args = args.collect::<Vec<_>>();
    match first.as_deref().and_then(|arg| arg.to_str()) {
        Some("--help" | "-h") if args.is_empty() => println!("{USAGE}"),
        Some("--version" | "-V") if args.is_empty() => {
            println!("pane-md {}", env!("CARGO_PKG_VERSION"));
        }
        Some("--mcp") if args.is_empty() => crate::mcp::run()?,
        Some("--list-comments") => {
            let paths = paths(args)?;
            let [path] = paths.as_slice() else {
                return Err(USAGE.into());
            };
            let doc = Markdown::parse(&read_document(path)?);
            println!("{}", comments_json(path, doc));
        }
        Some("--lint") => {
            let json = args.first().is_some_and(|arg| arg == "--json");
            let paths = paths(args.into_iter().skip(usize::from(json)))?;
            return lint_paths(&paths, json);
        }
        Some("--") => match args.as_slice() {
            [path] => open_gui(Some(&std::path::absolute(path)?))?,
            _ => return Err(USAGE.into()),
        },
        Some("--gui") if args.len() <= 1 => run_gui(args.pop().map(PathBuf::from), false)?,
        Some("--source") if args.len() == 2 && args[0] == "--wait" => {
            run_gui(args.pop().map(PathBuf::from), true)?;
        }
        Some(flag) if flag.starts_with('-') => return Err(USAGE.into()),
        _ if args.is_empty() => {
            let path = first.map(std::path::absolute).transpose()?;
            open_gui(path.as_deref())?;
        }
        _ => return Err(USAGE.into()),
    }
    Ok(ExitCode::SUCCESS)
}

fn paths(args: impl IntoIterator<Item = OsString>) -> Result<Vec<PathBuf>> {
    let mut args = args.into_iter().peekable();
    let literal = args.next_if(|arg| arg == "--").is_some();
    let paths = args
        .map(|arg| {
            if !literal && arg.to_str().is_some_and(|arg| arg.starts_with('-')) {
                return Err(USAGE.into());
            }
            Ok(std::path::absolute(arg)?)
        })
        .collect::<Result<Vec<_>>>()?;
    if paths.is_empty() {
        return Err(USAGE.into());
    }
    Ok(paths)
}

fn lint_paths(paths: &[PathBuf], as_json: bool) -> Result<ExitCode> {
    let mut reports = Vec::new();
    let mut clean = true;
    for path in paths {
        let source = read_document(path)?;
        let issues = lint::check_at(&source, path);
        clean &= issues.is_empty();
        if as_json {
            reports.push(json!({ "path": path.to_string_lossy(), "issues": issues }));
        } else {
            for issue in issues {
                eprintln!(
                    "{}:{}:{}: {}: {}",
                    path.display(),
                    issue.line,
                    issue.column,
                    issue.code,
                    issue.message
                );
            }
        }
    }
    if as_json {
        println!("{}", serde_json::to_string(&reports)?);
    }
    Ok(if clean {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub fn comments_json(path: &Path, doc: Markdown) -> serde_json::Value {
    json!({ "path": path.to_string_lossy(), "comments": doc.records, "diagnostics": doc.diagnostics })
}

pub fn read_document(path: &Path) -> io::Result<String> {
    let read = || {
        if !fs::metadata(path)?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected a regular file",
            ));
        }
        let bytes = fs::read(path)?;
        let (_, text) =
            decode(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(text.to_owned())
    };
    read().map_err(|error: io::Error| {
        io::Error::new(error.kind(), format!("{}: {error}", path.display()))
    })
}

#[cfg(windows)]
use crate::desktop as gui;
#[cfg(all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos")))]
use crate::portable as gui;

fn open_gui(path: Option<&Path>) -> Result<()> {
    let stderr = if cfg!(windows) {
        Stdio::null()
    } else {
        Stdio::inherit()
    };
    launch_gui(path, stderr)
}

#[cfg(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
))]
pub fn launch_gui(path: Option<&Path>, stderr: Stdio) -> Result<()> {
    #[cfg(all(target_os = "macos", feature = "desktop-gui"))]
    if path.is_none() && std::env::current_exe().is_ok_and(|exe| in_app_bundle(&exe)) {
        return run_gui(None, false);
    }
    if let Some(path) = path {
        let metadata = fs::metadata(path)?;
        if !metadata.is_file() && !metadata.is_dir() {
            return Err("expected a file or folder".into());
        }
    }
    gui::spawn_gui(path, stderr)?;
    Ok(())
}

#[cfg(any(all(target_os = "macos", feature = "desktop-gui"), test))]
fn in_app_bundle(executable: &Path) -> bool {
    executable.parent().is_some_and(|macos| {
        macos.file_name().is_some_and(|name| name == "MacOS")
            && macos.parent().is_some_and(|contents| {
                contents.file_name().is_some_and(|name| name == "Contents")
                    && contents
                        .parent()
                        .and_then(Path::extension)
                        .is_some_and(|extension| extension == "app")
            })
    })
}

#[cfg(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
))]
fn run_gui(path: Option<PathBuf>, wait: bool) -> Result<()> {
    gui::run_gui(path, wait)
}

#[cfg(not(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
)))]
pub fn launch_gui(_path: Option<&Path>, _stderr: Stdio) -> Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "this binary was built without the desktop editor",
    )
    .into())
}

#[cfg(not(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
)))]
fn run_gui(path: Option<PathBuf>, _wait: bool) -> Result<()> {
    open_gui(path.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finder_uses_the_bundle_process_and_cli_uses_the_launcher() {
        assert!(in_app_bundle(Path::new(
            "/Applications/pane-md.app/Contents/MacOS/pane-md"
        )));
        assert!(!in_app_bundle(Path::new("/usr/local/bin/pane-md")));
        assert!(!in_app_bundle(Path::new(
            "/tmp/package/Contents/MacOS/pane-md"
        )));
    }
}
