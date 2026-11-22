"""Build, sign and package a universal pane-md app on macOS."""

import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile

from release_files import ROOT, release_files


TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin"]
MINIMUM_MACOS = "13.0"
APP = "pane-md.app"
ICON = "pane-md.icns"
BINARY = Path("Contents/MacOS/pane-md")


def info(version):
    markdown_type = "net.daringfireball.markdown"
    return {
        "CFBundleIdentifier": "io.pane-md.editor",
        "CFBundleName": "pane-md",
        "CFBundleDisplayName": "pane-md",
        "CFBundleExecutable": BINARY.name,
        "CFBundleIconFile": ICON,
        "CFBundlePackageType": "APPL",
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleVersion": version,
        "CFBundleShortVersionString": version,
        "LSMinimumSystemVersion": MINIMUM_MACOS,
        "NSHighResolutionCapable": True,
        "CFBundleDocumentTypes": [{
            "CFBundleTypeName": "Markdown document",
            "CFBundleTypeRole": "Editor",
            "LSHandlerRank": "Alternate",
            "LSItemContentTypes": [markdown_type],
        }],
        "UTImportedTypeDeclarations": [{
            "UTTypeIdentifier": markdown_type,
            "UTTypeDescription": "Markdown document",
            "UTTypeConformsTo": ["public.plain-text"],
            "UTTypeTagSpecification": {
                "public.filename-extension": ["md", "markdown", "mdown", "mkd"],
            },
        }],
    }


def icon_resource(work, resources):
    source = ROOT / "assets" / "pane-md.png"
    iconset = work / "pane-md.iconset"
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            suffix = "@2x" if scale == 2 else ""
            image = iconset / f"icon_{size}x{size}{suffix}.png"
            pixels = str(size * scale)
            subprocess.run(
                ["sips", "-z", pixels, pixels, source, "--out", image],
                stdout=subprocess.DEVNULL, check=True,
            )
    subprocess.run(["iconutil", "-c", "icns", "-o", resources / ICON, iconset], check=True)


def prepare_bundle(stage, version, files, notices):
    app = stage / APP
    (app / BINARY.parent).mkdir(parents=True)
    resources = app / "Contents" / "Resources"
    resources.mkdir()
    (app / "Contents" / "Info.plist").write_bytes(plistlib.dumps(info(version)))
    for name, source in files.items():
        destination = stage / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
    (stage / "THIRD-PARTY.txt").write_bytes(notices.encode("utf-8"))
    return app, resources


def main():
    if sys.platform != "darwin":
        raise SystemExit("Build the macOS app on macOS with Xcode Command Line Tools installed.")
    target_dir, version, files, notices = release_files(TARGETS)
    env = dict(os.environ, MACOSX_DEPLOYMENT_TARGET=MINIMUM_MACOS)
    for target in TARGETS:
        subprocess.run(
            ["cargo", "build", "--release", "--locked", "--bin", "pane-md", "--target", target],
            cwd=ROOT, env=env, check=True,
        )
    release = target_dir / "release"
    release.mkdir(parents=True, exist_ok=True)
    archive = release / "pane-md-macos-universal.zip"
    with tempfile.TemporaryDirectory(dir=release) as temporary:
        work = Path(temporary)
        stage = work / "stage"
        app, resources = prepare_bundle(stage, version, files, notices)
        icon_resource(work, resources)
        slices = [target_dir / target / "release" / BINARY.name for target in TARGETS]
        subprocess.run(["xcrun", "lipo", "-create", *slices, "-output", app / BINARY], check=True)
        (app / BINARY).chmod(0o755)
        subprocess.run(["codesign", "--force", "--sign", "-", app], check=True)
        pending = work / archive.name
        subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", stage, pending], check=True)
        unpacked = work / "verification"
        subprocess.run(["ditto", "-x", "-k", pending, unpacked], check=True)
        app = unpacked / APP
        executable = app / BINARY
        archs = subprocess.check_output(["xcrun", "lipo", "-archs", executable], text=True)
        if set(archs.split()) != {"arm64", "x86_64"}:
            raise SystemExit("The archived executable must contain both Mac architectures.")
        subprocess.run(["codesign", "--verify", "--strict", "--all-architectures", app], check=True)
        if subprocess.check_output([executable, "--version"], text=True).strip() != f"pane-md {version}":
            raise SystemExit("The archived executable did not report the packaged version.")
        pending.replace(archive)
    print(archive)


if __name__ == "__main__":
    main()
