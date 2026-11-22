"""Render one CHANGELOG section as a GitHub release description."""

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]


def release_notes(tag, changelog):
    version = tag.removeprefix("v")
    lines = changelog.splitlines()
    start = next((i for i, line in enumerate(lines) if line.startswith(f"## {version} ")), None)
    if start is None:
        raise ValueError(f"CHANGELOG.md has no section for {tag}")
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("## ")), len(lines))
    changes = "\n".join(lines[start + 1:end]).strip()
    if not changes:
        raise ValueError(f"CHANGELOG.md has no changes for {tag}")
    return (f"# pane-md {tag}\n\n{changes}\n\n"
            "## Downloads\n\n"
            "- Windows x64: `pane-md-windows-x64.zip`\n"
            "- Linux x64: `pane-md-linux-x64.tar.gz`\n"
            "- macOS ARM64 + x64: `pane-md-macos-universal.zip`\n"
            "- Integrity: `SHA256SUMS.txt`\n\n"
            f"[English README](https://github.com/letya999/pane-md/blob/{tag}/README.md) · "
            f"[Русское описание](https://github.com/letya999/pane-md/blob/{tag}/README.ru.md)\n")


if __name__ == "__main__":
    if len(sys.argv) != 2 or not sys.argv[1].startswith("v"):
        raise SystemExit("Usage: release-notes.py vX.Y.Z")
    try:
        print(release_notes(sys.argv[1], (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")))
    except ValueError as error:
        raise SystemExit(str(error)) from error
