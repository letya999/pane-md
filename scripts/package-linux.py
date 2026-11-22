"""Package the existing Linux x86-64 release with its icon and notices."""

import io
from pathlib import Path
import tarfile
import tempfile

from release_files import ROOT, release_files


def main():
    target_dir, _, files, notices = release_files(["x86_64-unknown-linux-gnu"])
    release = target_dir / "release"
    executable = release / "pane-md"
    if not executable.is_file():
        raise SystemExit("Run cargo build --release --locked on Linux x86-64 first.")
    with executable.open("rb") as candidate:
        header = candidate.read(20)
    if not (header.startswith(b"\x7fELF\x02\x01") and header[18:20] == b"\x3e\x00"):
        raise SystemExit("The release executable must be a Linux x86-64 ELF file.")
    files[executable.name] = executable
    files["pane-md.png"] = ROOT / "assets" / "pane-md.png"
    archive = release / "pane-md-linux-x64.tar.gz"
    with tempfile.TemporaryDirectory(dir=release) as temporary:
        pending = Path(temporary) / archive.name
        with tarfile.open(pending, "w:gz") as tar:
            for name, file in files.items():
                tar.add(file, name, recursive=False)
            notice = notices.encode("utf-8")
            entry = tarfile.TarInfo("THIRD-PARTY.txt")
            entry.size = len(notice)
            tar.addfile(entry, io.BytesIO(notice))
        pending.replace(archive)
    print(archive)


if __name__ == "__main__":
    main()
