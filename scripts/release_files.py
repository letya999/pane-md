"""Shared release metadata and dependency notices."""

import json
from pathlib import Path
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def license_files(package):
    folder = Path(package["manifest_path"]).parent
    for entry in sorted(folder.iterdir()):
        if entry.name.upper().startswith(("LICENSE", "NOTICE", "COPYING", "THIRD_PARTY")):
            files = sorted(entry.rglob("*")) if entry.is_dir() else [entry]
            yield from ((file, file.relative_to(folder)) for file in files if file.is_file())
    if package["name"] == "merman-elk-layered":
        yield folder / "README.md", Path("ELK-PROVENANCE.md")


def release_files(targets):
    packages = {}
    for target in targets:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--locked", "--format-version", "1",
             "--filter-platform", target], cwd=ROOT,
        ))
        packages.update((package["id"], package) for package in metadata["packages"])
    version = packages[metadata["resolve"]["root"]]["version"]
    files = {
        "README.md": ROOT / "README.md",
        "README.ru.md": ROOT / "README.ru.md",
        "docs/guide.ru.md": ROOT / "docs" / "guide.ru.md",
        "LICENSE": ROOT / "LICENSE",
        "Cargo.lock": ROOT / "Cargo.lock",
    }
    notices = ["Third-party dependencies and corresponding source archives", ""]
    for package in sorted(packages.values(), key=lambda item: (item["name"], item["version"])):
        if not package["source"]:
            continue
        name, release = package["name"], package["version"]
        notices.extend([
            f'{name} {release} ({package["license"] or package["license_file"]})',
            f"https://crates.io/api/v1/crates/{name}/{release}/download",
        ])
        for file, relative in license_files(package):
            files[f"licenses/{name}-{release}/{relative.as_posix()}"] = file
    return Path(metadata["target_directory"]), version, files, "\n".join(notices) + "\n"
