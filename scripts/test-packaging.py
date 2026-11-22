"""Verify the portable parts of the release packagers."""

from pathlib import Path
import plistlib
from runpy import run_path
import tempfile
import unittest

from release_files import ROOT, license_files

release_notes = run_path(str(ROOT / "scripts" / "release-notes.py"))["release_notes"]


class PackagingTests(unittest.TestCase):
    def test_release_notes_select_one_version_and_require_changes(self):
        changelog = (ROOT / "docs/CHANGELOG.md").read_text(encoding="utf-8")
        first = release_notes("v0.1.0", changelog)
        latest = release_notes("v0.1.1", changelog)
        self.assertIn("STDIO MCP server", first)
        self.assertNotIn("Windows Explorer version details", first)
        self.assertIn("Windows Explorer version details", latest)
        with self.assertRaises(ValueError):
            release_notes("v9.9.9", changelog)

    def test_mac_bundle_layout_and_markdown_association(self):
        package = run_path(str(ROOT / "scripts" / "package-macos.py"))
        with tempfile.TemporaryDirectory() as folder:
            stage = Path(folder)
            files = {"README.md": ROOT / "README.md", "Cargo.lock": ROOT / "Cargo.lock"}
            app, resources = package["prepare_bundle"](stage, "0.1.0", files, "notices\n")
            info = plistlib.loads((app / "Contents" / "Info.plist").read_bytes())
            self.assertEqual(app.name, "pane-md.app")
            self.assertEqual(resources, app / "Contents" / "Resources")
            self.assertEqual(info["CFBundleExecutable"], "pane-md")
            self.assertEqual(info["CFBundleIconFile"], "pane-md.icns")
            self.assertEqual(info["NSHumanReadableCopyright"], "Copyright (c) 2026 Artem Letyushev")
            self.assertEqual(info["LSMinimumSystemVersion"], package["MINIMUM_MACOS"])
            document, = info["CFBundleDocumentTypes"]
            declaration, = info["UTImportedTypeDeclarations"]
            self.assertEqual(document["LSHandlerRank"], "Alternate")
            self.assertEqual(document["LSItemContentTypes"], [declaration["UTTypeIdentifier"]])
            self.assertEqual(
                declaration["UTTypeTagSpecification"]["public.filename-extension"],
                ["md", "markdown", "mdown", "mkd"],
            )
            self.assertTrue((app / "Contents" / "MacOS").is_dir())
            self.assertEqual((stage / "README.md").read_bytes(), files["README.md"].read_bytes())
            self.assertEqual((stage / "THIRD-PARTY.txt").read_bytes(), b"notices\n")

    def test_dependency_licenses_keep_elk_provenance(self):
        with tempfile.TemporaryDirectory() as folder:
            package = Path(folder)
            (package / "LICENSES").mkdir()
            (package / "LICENSES" / "LICENSE-MIT").touch()
            (package / "README.md").touch()
            self.assertEqual(
                [name.as_posix() for _, name in license_files({
                    "name": "merman-elk-layered", "manifest_path": package / "Cargo.toml",
                })],
                ["LICENSES/LICENSE-MIT", "ELK-PROVENANCE.md"],
            )


if __name__ == "__main__":
    unittest.main()
