"""Package the Windows executable with dependency licenses and source links."""

from pathlib import Path
import tempfile
import zipfile

from release_files import release_files


def pe_subsystem(path):
    with path.open("rb") as binary:
        binary.seek(0x3C)
        header = int.from_bytes(binary.read(4), "little")
        binary.seek(header + 92)
        return int.from_bytes(binary.read(2), "little")


def main():
    target_dir, _, files, notices = release_files(["x86_64-pc-windows-msvc"])
    release = target_dir / "release"
    executable = release / "pane-md.exe"
    editor = release / "pane-md-editor.exe"
    archive = release / "pane-md-windows-x64.zip"
    if not executable.is_file() or not editor.is_file():
        raise SystemExit("Run cargo build --release --locked for x86_64-pc-windows-msvc first.")
    if pe_subsystem(executable) != 3 or pe_subsystem(editor) != 2:
        raise SystemExit("Windows release needs a console CLI and a GUI external-editor launcher.")
    with tempfile.TemporaryDirectory(dir=release) as temporary:
        pending = Path(temporary) / archive.name
        with zipfile.ZipFile(pending, "w", zipfile.ZIP_DEFLATED, strict_timestamps=False) as zipped:
            zipped.write(executable, executable.name)
            zipped.write(editor, editor.name)
            for name, file in files.items():
                zipped.write(file, name)
            zipped.writestr("THIRD-PARTY.txt", notices)
        pending.replace(archive)
    print(archive)


if __name__ == "__main__":
    main()
