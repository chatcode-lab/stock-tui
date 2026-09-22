from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "publish-crate.yml"
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
CONTENT_CHECK = ROOT / "tools" / "release" / "verify-crate-contents.sh"


class CrateReleasePolicyTests(unittest.TestCase):
    def test_package_gate_binds_release_tag_commit_and_workflow(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn('git cat-file -t "$RELEASE_TAG"', workflow)
        self.assertIn(
            "+refs/heads/main:refs/remotes/origin/main",
            workflow,
        )
        self.assertIn('release_commit="$(git rev-list -n 1 "$RELEASE_TAG")"', workflow)
        self.assertIn(
            'git merge-base --is-ancestor "$release_commit" origin/main',
            workflow,
        )
        self.assertIn(".draft == false and .prerelease == false", workflow)
        self.assertIn("--workflow release.yml", workflow)
        self.assertIn("--event push", workflow)
        self.assertIn(".headSha == $commit", workflow)

    def test_package_gate_checks_contents_dry_run_and_size(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertIn("cargo package --locked --list", workflow)
        self.assertIn("tools/release/verify-crate-contents.sh", workflow)
        self.assertIn("cargo publish --dry-run --locked --registry crates-io", workflow)
        self.assertIn('MAX_CRATE_SIZE_BYTES: "10485760"', workflow)

        release_workflow = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("tools/release/verify-crate-contents.sh", release_workflow)
        self.assertIn("cargo publish --dry-run --locked --registry crates-io", release_workflow)
        self.assertIn("crate_size > 10485760", release_workflow)

    def test_publish_paths_use_protected_environment_and_distinct_auth(self) -> None:
        workflow = WORKFLOW.read_text(encoding="utf-8")

        self.assertEqual(workflow.count("environment: crates-io"), 2)
        self.assertIn("CARGO_REGISTRY_TOKEN: ${{ secrets.CRATES_TOKEN }}", workflow)
        self.assertIn("id-token: write", workflow)
        self.assertIn("rust-lang/crates-io-auth-action@", workflow)
        self.assertIn("CARGO_REGISTRY_TOKEN: ${{ steps.crates-auth.outputs.token }}", workflow)
        self.assertNotIn("pull_request_target", workflow)

    def test_content_gate_allows_examples_and_rejects_runtime_secrets(self) -> None:
        safe = "\n".join((".env.example", "config.example.toml", "src/main.rs"))
        unsafe = "\n".join(
            (
                ".env.local",
                "docs/.env-production",
                "data/cache.db-wal",
                "state/market.sqlite-shm",
                "AuthKey_release.p8",
                "logs-provider.json",
            )
        )

        with tempfile.TemporaryDirectory() as temporary_directory:
            file_list = Path(temporary_directory) / "package-files.txt"
            file_list.write_text(f"{safe}\n", encoding="utf-8")
            subprocess.run([CONTENT_CHECK, file_list], cwd=ROOT, check=True)

            file_list.write_text(f"{safe}\n{unsafe}\n", encoding="utf-8")
            result = subprocess.run(
                [CONTENT_CHECK, file_list],
                cwd=ROOT,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            for path in unsafe.splitlines():
                with self.subTest(path=path):
                    self.assertIn(path, result.stderr)


if __name__ == "__main__":
    unittest.main()
