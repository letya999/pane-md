#[cfg(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
))]
mod appearance;
mod cli;
#[cfg(windows)]
mod desktop;
#[cfg(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
))]
mod diagram;
#[cfg(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
))]
mod document;
#[cfg(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
))]
mod i18n;
#[cfg(any(
    windows,
    all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos"))
))]
mod images;
mod mcp;
#[cfg(all(feature = "desktop-gui", any(target_os = "linux", target_os = "macos")))]
mod portable;

use std::process::ExitCode;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() -> ExitCode {
    match cli::run(std::env::args_os().skip(1)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("pane-md: {error}");
            ExitCode::from(2)
        }
    }
}
