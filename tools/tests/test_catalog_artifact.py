from __future__ import annotations

import gzip
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from tools import build_sec_catalog as catalog


SCRIPT = Path(__file__).resolve().parents[1] / "build_sec_catalog.py"


def sample_catalog() -> dict[str, object]:
    return {
        "schema_version": 2,
        "catalog_version": "sec-universe-v2-fixture",
        "generated_at": "2026-07-25T00:00:00Z",
        "as_of": "2026-07-24",
        "selection": {"audit_only": True},
        "sources": [{"id": "large-source-receipt"}],
        "companies": [
            {
                "rank": 1,
                "cik": "0000000001",
                "symbol": "ONE",
                "name": "One Corp.",
                "exchange": "Nasdaq",
                "sic": 3571,
                "sector": "technology",
                "public_float": 1_000_000_000,
                "proxy_source": "sec_public_float",
                "proxy_as_of": "2026-06-30",
                "proxy_confidence": "low",
                "proxy_sanity_screen": "audit-only-screen",
                "shares_outstanding": 10_000_000,
                "shares_source": "sec_shares",
                "shares_as_of": "2026-07-24",
                "shares_method": "sec_frame_dei_total",
                "shares_confidence": "high",
                "as_of": "2026-06-30",
                "quality": "public_float_and_shares",
                "provenance": {
                    "identity": {"source": "sec_tickers"},
                    "sic": {
                        "source": "sec_fsds",
                        "accession": "fixture-accession",
                    },
                    "public_float": {
                        "source": "sec_public_float",
                        "accession": "fixture-accession",
                        "frame": "CY2026Q2I",
                        "end": "2026-06-30",
                        "confidence": "low",
                        "sanity_screen": "audit-only-screen",
                    },
                    "shares_outstanding": {
                        "source": "sec_shares",
                        "accession": "fixture-accession",
                        "end": "2026-07-24",
                        "method": "sec_frame_dei_total",
                        "confidence": "high",
                        "frame": "CY2026Q2I",
                        "components": [{"audit_only": True}],
                    },
                },
            },
            {
                "rank": 101,
                "cik": "0000000002",
                "symbol": "TWO",
                "name": "Two Corp.",
                "exchange": "NYSE",
                "sic": 7372,
                "sector": "technology",
                "public_float": 500_000_000,
                "shares_outstanding": None,
                "as_of": "2026-03-31",
                "quality": "public_float_only",
                "provenance": {
                    "public_float": {
                        "source": "sec_public_float",
                        "end": "2026-03-31",
                        "confidence": "low",
                    },
                    "shares_outstanding": None,
                },
            },
        ],
    }


class CatalogArtifactTests(unittest.TestCase):
    def test_runtime_projection_retains_client_fields_and_drops_audit_data(
        self,
    ) -> None:
        runtime = catalog.runtime_catalog(sample_catalog())

        self.assertEqual(
            set(runtime),
            {
                "schema_version",
                "catalog_version",
                "generated_at",
                "as_of",
                "companies",
            },
        )
        company = runtime["companies"][0]
        self.assertEqual(
            set(company),
            {
                "rank",
                "cik",
                "symbol",
                "name",
                "exchange",
                "sic",
                "sector",
                "public_float",
                "shares_outstanding",
                "as_of",
                "quality",
                "provenance",
            },
        )
        self.assertEqual(
            company["provenance"],
            {
                "public_float": {
                    "source": "sec_public_float",
                    "end": "2026-06-30",
                    "confidence": "low",
                },
                "shares_outstanding": {
                    "source": "sec_shares",
                    "end": "2026-07-24",
                    "method": "sec_frame_dei_total",
                    "confidence": "high",
                },
            },
        )
        self.assertIsNone(
            runtime["companies"][1]["provenance"]["shares_outstanding"]
        )

    def test_gzip_and_manifest_are_byte_for_byte_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first_artifact = root / "first" / "sec-catalog.json.gz"
            second_artifact = root / "second" / "sec-catalog.json.gz"

            first_manifest, first_manifest_path = (
                catalog.write_runtime_catalog_artifact(
                    sample_catalog(), first_artifact
                )
            )
            second_manifest, second_manifest_path = (
                catalog.write_runtime_catalog_artifact(
                    sample_catalog(), second_artifact
                )
            )

            compressed = first_artifact.read_bytes()
            self.assertEqual(compressed, second_artifact.read_bytes())
            self.assertEqual(
                first_manifest_path.read_bytes(),
                second_manifest_path.read_bytes(),
            )
            self.assertEqual(first_manifest, second_manifest)
            self.assertEqual(compressed[3], 0)
            self.assertEqual(compressed[4:8], b"\0\0\0\0")

            payload = gzip.decompress(compressed)
            self.assertEqual(
                payload,
                catalog.canonical_json_bytes(
                    catalog.runtime_catalog(sample_catalog())
                ),
            )
            artifact = first_manifest["artifact"]
            self.assertEqual(artifact["content_type"], "application/json")
            self.assertEqual(artifact["content_encoding"], "gzip")
            self.assertEqual(
                artifact["sha256"], hashlib.sha256(compressed).hexdigest()
            )
            self.assertEqual(
                artifact["payload_sha256"],
                hashlib.sha256(payload).hexdigest(),
            )
            self.assertEqual(
                first_manifest_path.name, "sec-catalog.manifest.json"
            )

    def test_package_only_cli_does_not_require_sec_credentials_or_cache(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "audit-catalog.json"
            artifact = root / "sec-catalog.json.gz"
            source.write_text(json.dumps(sample_catalog()), encoding="utf-8")
            environment = os.environ.copy()
            environment.pop("SEC_USER_AGENT", None)

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--package-only",
                    "--output",
                    str(source),
                    "--artifact-output",
                    str(artifact),
                ],
                cwd=SCRIPT.parents[1],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(artifact.is_file())
            self.assertTrue((root / "sec-catalog.manifest.json").is_file())
            self.assertNotIn(
                "selection",
                json.loads(gzip.decompress(artifact.read_bytes())),
            )
            self.assertEqual(
                sorted(path.name for path in root.iterdir()),
                [
                    "audit-catalog.json",
                    "sec-catalog.json.gz",
                    "sec-catalog.manifest.json",
                ],
            )

    def test_package_only_rejects_an_unresolved_top_100_company(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "audit-catalog.json"
            artifact = root / "sec-catalog.json.gz"
            payload = sample_catalog()
            payload["companies"][1]["rank"] = 2
            source.write_text(json.dumps(payload), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--package-only",
                    "--output",
                    str(source),
                    "--artifact-output",
                    str(artifact),
                ],
                cwd=SCRIPT.parents[1],
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("top-100 share coverage regression", result.stderr)
            self.assertFalse(artifact.exists())

    def test_shares_value_and_provenance_must_match(self) -> None:
        source = sample_catalog()
        source["companies"][0]["provenance"]["shares_outstanding"] = None

        with self.assertRaisesRegex(
            RuntimeError,
            "shares value and provenance must both be present or absent",
        ):
            catalog.runtime_catalog(source)


if __name__ == "__main__":
    unittest.main()
