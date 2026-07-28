from __future__ import annotations

import os
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
SIGNING_SCRIPT = ROOT / "tools" / "release" / "sign-notarize-macos.sh"


class MacosReleasePolicyTests(unittest.TestCase):
    def test_macos_job_is_protected_and_required_for_publication(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("build-macos:", workflow)
        self.assertIn("environment: macos-release", workflow)
        self.assertIn("timeout-minutes: 120", workflow)
        self.assertNotIn("runner.os == 'macOS' && github.ref_type == 'tag'", workflow)
        non_macos_build = workflow.split("\n  build:\n", maxsplit=1)[1].split(
            "\n  build-macos:\n",
            maxsplit=1,
        )[0]
        macos_build = workflow.split("\n  build-macos:\n", maxsplit=1)[1].split(
            "\n  publish:\n",
            maxsplit=1,
        )[0]
        self.assertLess(
            workflow.index("name: Sign and notarize macOS release"),
            workflow.index("name: Archive signed macOS executable"),
        )
        self.assertIn("needs: [build, build-macos]", workflow)
        self.assertIn("pattern: stock-tui-*", workflow)
        self.assertIn("Expected exactly 5 platform archives", workflow)
        self.assertIn(
            "APPLE_NOTARY_API_KEY_P8_BASE64: ${{ secrets.",
            workflow,
        )
        self.assertNotIn("APPLE_NOTARY_API_KEY_P8_BASE64", non_macos_build)
        self.assertIn("APPLE_NOTARY_API_KEY_P8_BASE64", macos_build)
        self.assertIn(
            "MACOS_SIGNING_CERT_P12_BASE64: ${{ secrets.",
            workflow,
        )
        self.assertNotIn("MACOS_SIGNING_CERT_P12_BASE64", non_macos_build)
        self.assertIn("MACOS_SIGNING_CERT_P12_BASE64", macos_build)
        self.assertNotIn(".dmg", workflow)
        self.assertIn(
            "path: stock-tui-v*-${{ matrix.target }}.tar.gz",
            macos_build,
        )
        self.assertIn(
            'cmp -s dist/stock-tui "$verification_directory/stock-tui"',
            macos_build,
        )
        self.assertIn(
            "Archived executable satisfies Apple notarization requirement.",
            macos_build,
        )
        self.assertIn("--check-notarization -R='notarized'", macos_build)
        self.assertNotIn("spctl", macos_build)

    def test_release_script_enforces_apple_distribution_controls(self) -> None:
        script = SIGNING_SCRIPT.read_text(encoding="utf-8")

        self.assertTrue(os.access(SIGNING_SCRIPT, os.X_OK))
        for required_command in (
            "--options runtime",
            "--timestamp",
            "codesign --display --entitlements :-",
            "/usr/bin/ditto -c -k --keepParent",
            "xcrun notarytool submit",
            "xcrun notarytool log",
            "--check-notarization",
            "-R='notarized'",
        ):
            with self.subTest(required_command=required_command):
                self.assertIn(required_command, script)

        self.assertIn("Developer ID Application:", script)
        self.assertIn("/usr/bin/base64 -D", script)
        self.assertIn("--output-format json", script)
        self.assertGreaterEqual(script.count("--output-format json"), 2)
        self.assertIn("plutil", script)
        self.assertIn("/usr/libexec/PlistBuddy", script)
        self.assertIn("com.apple.security.get-task-allow", script)
        self.assertIn("original_keychains=()", script)
        self.assertGreaterEqual(script.count("security list-keychains -d user -s"), 2)
        self.assertIn('notary_status" != "Accepted"', script)
        self.assertIn('notary_log_status" != "Accepted"', script)
        self.assertIn('notary_error_count" -ne 0', script)
        self.assertIn("Online notarization requirement accepted.", script)
        self.assertIn("for attempt in 1 2 3 4 5 6", script)
        self.assertIn('sleep 10', script)
        self.assertIn("trap cleanup EXIT HUP INT TERM", script)
        self.assertNotIn("stapler", script)
        self.assertNotIn("hdiutil", script)
        self.assertNotIn("spctl", script)
        self.assertLess(
            script.index("/usr/bin/ditto -c -k --keepParent"),
            script.index("xcrun notarytool submit"),
        )
        self.assertLess(
            script.index("xcrun notarytool submit"),
            script.index("xcrun notarytool log"),
        )
        self.assertLess(
            script.index("xcrun notarytool log"),
            script.index("notarization_accepted="),
        )


if __name__ == "__main__":
    unittest.main()
