#[cfg(windows)]
mod desktop;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    match desktop::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("notepad-md: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The notepad-md desktop application requires Windows.");
}
