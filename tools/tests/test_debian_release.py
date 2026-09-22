from __future__ import annotations

import gzip
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
PACKAGING_SCRIPT = ROOT / "tools" / "release" / "build-deb.sh"


class DebianReleasePolicyTests(unittest.TestCase):
    def test_native_linux_jobs_build_and_smoke_test_packages(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("deb_arch: amd64", workflow)
        self.assertIn("deb_arch: arm64", workflow)
        self.assertIn("tools/release/build-deb.sh", workflow)
        self.assertIn('sudo dpkg --install "$package"', workflow)
        self.assertIn("sudo dpkg --remove stock-tui", workflow)
        self.assertIn("test ! -e /usr/bin/stock-tui", workflow)
        self.assertIn("test ! -e /usr/share/doc/stock-tui", workflow)
        self.assertIn("stock-tui_*.deb", workflow)

    def test_release_requires_exact_archive_and_debian_asset_set(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        expected_assets = (
            "aarch64-apple-darwin.tar.gz",
            "aarch64-unknown-linux-musl.tar.gz",
            "x86_64-apple-darwin.tar.gz",
            "x86_64-pc-windows-msvc.zip",
            "x86_64-unknown-linux-musl.tar.gz",
            "amd64.deb",
            "arm64.deb",
        )
        for asset in expected_assets:
            with self.subTest(asset=asset):
                self.assertIn(asset, workflow)
        self.assertIn("${#files[@]} != ${#expected[@]}", workflow)
        self.assertIn('[[ ! -f "$expected_file" ]]', workflow)

    def test_packaging_script_installs_policy_files(self) -> None:
        script = PACKAGING_SCRIPT.read_text(encoding="utf-8")

        self.assertTrue(os.access(PACKAGING_SCRIPT, os.X_OK))
        self.assertIn("$package_root/usr/bin/stock-tui", script)
        self.assertIn("$documentation/copyright", script)
        self.assertIn("$documentation/changelog.gz", script)
        self.assertIn("$documentation/README.md", script)
        self.assertIn("$documentation/examples/config.toml", script)
        self.assertIn("$documentation/examples/environment", script)
        self.assertIn("dpkg-deb --root-owner-group --build", script)
        self.assertIn("Package: stock-tui", script)
        self.assertIn("Architecture: $architecture", script)

    @unittest.skipUnless(shutil.which("dpkg-deb"), "dpkg-deb is not installed")
    def test_packaging_script_builds_expected_layout(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            temporary = Path(temporary_directory)
            binary = temporary / "stock-tui"
            binary.write_text("#!/bin/sh\nprintf 'stock-tui 1.2.3\\n'\n", encoding="utf-8")
            binary.chmod(0o755)

            subprocess.run(
                [PACKAGING_SCRIPT, binary, "1.2.3", "amd64", temporary],
                cwd=ROOT,
                check=True,
                capture_output=True,
                text=True,
            )

            package = temporary / "stock-tui_1.2.3_amd64.deb"
            extracted = temporary / "extracted"
            subprocess.run(
                ["dpkg-deb", "--extract", package, extracted],
                check=True,
                capture_output=True,
                text=True,
            )
            fields = subprocess.run(
                ["dpkg-deb", "--field", package],
                check=True,
                capture_output=True,
                text=True,
            ).stdout

            documentation = extracted / "usr/share/doc/stock-tui"
            self.assertIn("Package: stock-tui\n", fields)
            self.assertIn("Version: 1.2.3\n", fields)
            self.assertIn("Architecture: amd64\n", fields)
            self.assertEqual((extracted / "usr/bin/stock-tui").stat().st_mode & 0o777, 0o755)
            self.assertEqual(
                (documentation / "README.md").read_bytes(),
                (ROOT / "README.md").read_bytes(),
            )
            self.assertEqual(
                (documentation / "copyright").read_bytes(),
                (ROOT / "LICENSE").read_bytes(),
            )
            self.assertEqual(
                gzip.decompress((documentation / "changelog.gz").read_bytes()),
                (ROOT / "CHANGELOG.md").read_bytes(),
            )
            self.assertEqual(
                (documentation / "examples/config.toml").read_bytes(),
                (ROOT / "config.example.toml").read_bytes(),
            )
            self.assertEqual(
                (documentation / "examples/environment").read_bytes(),
                (ROOT / ".env.example").read_bytes(),
            )


if __name__ == "__main__":
    unittest.main()
