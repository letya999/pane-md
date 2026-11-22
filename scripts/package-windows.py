"""Package the Windows executable with dependency licenses and source links."""

from pathlib import Path
import tempfile
import zipfile

from release_files import release_files


def main():
    target_dir, _, files, notices = release_files(["x86_64-pc-windows-msvc"])
    release = target_dir / "release"
    executable = release / "pane-md.exe"
    archive = release / "pane-md-windows-x64.zip"
    if not executable.is_file():
        raise SystemExit("Run cargo build --release --locked for x86_64-pc-windows-msvc first.")
    with tempfile.TemporaryDirectory(dir=release) as temporary:
        pending = Path(temporary) / archive.name
        with zipfile.ZipFile(pending, "w", zipfile.ZIP_DEFLATED, strict_timestamps=False) as zipped:
            zipped.write(executable, executable.name)
            for name, file in files.items():
                zipped.write(file, name)
            zipped.writestr("THIRD-PARTY.txt", notices)
        pending.replace(archive)
    print(archive)


if __name__ == "__main__":
    main()
