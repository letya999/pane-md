# Contributing to pane-md

Thanks for helping improve pane-md. Small, focused changes with a clear reproduction are easiest to review. Use the bug and feature issue forms for public discussion; use [private reporting](SECURITY.md) for vulnerabilities.

Please follow the [code of conduct](CODE_OF_CONDUCT.md).

## Set up

Install Rust 1.95 or newer. On Windows, install Visual Studio Build Tools and Windows SDK. On Linux, install the GTK 3 and WebKitGTK 4.1 development packages. On macOS, install Xcode Command Line Tools. Build with:

```sh
cargo build --locked
```

For CLI and MCP work on Linux or macOS without GUI libraries, use `--no-default-features`.

## Check a change

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
python scripts/test-packaging.py
```

The CI matrix runs on Windows, Linux, and macOS. Browser component checks run on Linux. The Windows GUI smoke script and Linux X11 smoke script are documented in the [detailed guide](docs/guide.ru.md). Run the relevant one when changing window behavior. A macOS build check does not replace a manual GUI test on a Mac.

## Pull requests

- Describe the user-visible change, the affected platform, and how you verified it.
- Include a regression test for a behavior change that could corrupt files, comments, links, or MCP output.
- Keep [README.md](README.md) and [README.ru.md](README.ru.md) consistent when changing documented behavior.
- Preserve existing Markdown bytes and review anchors where possible. Explain intentional format migrations.
- Do not commit real documents, tokens, machine-specific paths, generated build files, or secrets.

The project uses the [MIT License](LICENSE). By submitting a contribution, you agree to license it under MIT.
