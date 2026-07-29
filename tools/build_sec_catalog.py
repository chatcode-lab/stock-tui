#!/usr/bin/env python3
"""Build stock-tui's embedded issuer universe from official SEC data.

The output deliberately stores SEC EntityPublicFloat as a ranking proxy, not as
market capitalization. A runtime market cap requires a contemporaneous price
multiplied by shares outstanding.
"""

from __future__ import annotations

import argparse
import csv
import gzip
import hashlib
import io
import json
import math
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile
import xml.etree.ElementTree as ElementTree
from dataclasses import dataclass
from datetime import date, datetime, timezone
from pathlib import Path
from typing import Any, Iterable


TICKERS_URL = "https://www.sec.gov/files/company_tickers_exchange.json"
SUBMISSIONS_URL = "https://data.sec.gov/submissions/CIK{cik:010d}.json"
FILING_ARCHIVE_URL = (
    "https://www.sec.gov/Archives/edgar/data/{cik}/{accession}/{filename}"
)
FSDS_URL = (
    "https://www.sec.gov/files/dera/data/financial-statement-data-sets/"
    "{year}q{quarter}.zip"
)
FRAME_URL = (
    "https://data.sec.gov/api/xbrl/frames/{taxonomy}/{tag}/{unit}/"
    "CY{year}Q{quarter}I.json"
)
PUBLIC_FLOAT_TAG = "EntityPublicFloat"
SHARES_TAG = "EntityCommonStockSharesOutstanding"
COMMON_SHARES_TAG = "CommonStockSharesOutstanding"
BASIC_WEIGHTED_SHARES_TAG = "WeightedAverageNumberOfSharesOutstandingBasic"
LIMITED_PARTNERS_WEIGHTED_UNITS_TAG = (
    "WeightedAverageLimitedPartnershipUnitsOutstanding"
)
ELIGIBLE_FILING_FORMS = frozenset({"10-K", "10-Q", "20-F", "40-F"})
SECTORS = (
    "consumer",
    "services",
    "healthcare",
    "energy",
    "technology",
    "financial",
    "industrial",
    "materials",
    "utilities",
)
SCHEMA_VERSION = 2
ARTIFACT_MANIFEST_VERSION = 1
MIN_COMPANIES_PER_SECTOR = 100
TARGET_COMPANIES_PER_SECTOR = 250
MAX_REPORTED_PUBLIC_FLOAT = 5_000_000_000_000
MAX_POINT_FACT_OVERRIDE_DAYS = 45
MAX_WEIGHTED_FALLBACK_OVERRIDE_DAYS = 185
MAX_SHARE_FACT_AGE_DAYS = 550
MAX_UNREVIEWED_IMPLIED_SHARE_PRICE = 2_000
# Gross-error guards set 100x above the SEC filer-status float boundaries. The
# margin accommodates transition timing without rejecting legitimate issuers.
MAX_ACCELERATED_FILER_FLOAT = 70_000_000_000
MAX_NON_ACCELERATED_FILER_FLOAT = 7_500_000_000
REVIEWED_HIGH_PRICE_CIKS = {
    866787: "AutoZone common stock",
    1099590: "MercadoLibre common stock",
    906163: "NVR common stock",
    1067983: "Berkshire Hathaway Class A or filer-reported equivalent",
}
REVIEWED_LARGE_FLOAT_WITHOUT_AFS_CIKS: frozenset[int] = frozenset()
SHARE_POLICY_PATH = (
    Path(__file__).resolve().parents[1] / "data" / "sec_share_policies.json"
)
SHARE_POLICY_PRICE_BASES = frozenset(
    {
        "canonical_common_unit",
        "canonical_listed_class_scope",
        "canonical_symbol_proxy",
        "fully_converted_canonical_symbol_proxy",
        "multi_class_canonical_symbol_proxy",
        "provider_total_common_proxy",
        "public_and_founder_common_proxy",
    }
)
XBRLI_NAMESPACE = "http://www.xbrl.org/2003/instance"
XBRLDI_NAMESPACE = "http://xbrl.org/2006/xbrldi"
DEI_NAMESPACE_PREFIXES = (
    "http://xbrl.sec.gov/dei/",
    "https://xbrl.sec.gov/dei/",
)
US_GAAP_NAMESPACE_PREFIXES = (
    "http://fasb.org/us-gaap/",
    "https://fasb.org/us-gaap/",
)

# Canonical display/provider symbols with issuer-specific economic review.
# Molson Coors Class A converts one-for-one into Class B and shares its dividend
# economics, while Class B is the substantially larger listed class. Voting
# rights still differ, so this exception must remain scoped to this CIK.
# https://www.sec.gov/Archives/edgar/data/24545/000002454526000006/tap-20251231.htm
REVIEWED_CANONICAL_SYMBOLS = {
    24545: "TAP",
}

# These issuers have reviewed classes with equivalent per-share economics for
# market-cap purposes. The exact member set is deliberate: a new or renamed
# class makes the aggregation fail closed until the policy is reviewed.
REVIEWED_EQUAL_CLASS_MEMBERS = {
    320187: frozenset({"CommonClassA", "CommonClassB"}),  # NIKE
    1141391: frozenset({"CommonClassA", "CommonClassB"}),  # Mastercard
    1321655: frozenset(
        {"CommonClassA", "CommonClassB", "CommonClassF"}
    ),  # Palantir
    1326801: frozenset({"CommonClassA", "CommonClassB"}),  # Meta
    1652044: frozenset(
        {"CommonClassA", "CommonClassB", "CapitalClassC"}
    ),  # Alphabet
}
REVIEWED_EQUAL_CLASS_POLICY_METADATA = {
    320187: {
        "basis": "one-to-one common-share economic equivalent",
        "policy_source": (
            "https://www.sec.gov/Archives/edgar/data/320187/"
            "000032018725000151/nke-20251130.htm"
        ),
    },
    1141391: {
        "basis": "one-to-one common-share economic equivalent",
        "policy_source": (
            "https://www.sec.gov/Archives/edgar/data/1141391/"
            "000114139126000013/ma-20251231.htm"
        ),
    },
    1321655: {
        "basis": "one-to-one common-share economic equivalent",
        "policy_source": (
            "https://www.sec.gov/Archives/edgar/data/1321655/"
            "000132165526000011/pltr-20251231.htm"
        ),
    },
    1326801: {
        "basis": "one-to-one common-share economic equivalent",
        "policy_source": (
            "https://www.sec.gov/Archives/edgar/data/1326801/"
            "000162828026003942/meta-20251231.htm"
        ),
    },
    1652044: {
        "basis": "one-to-one common-share economic equivalent",
        "policy_source": (
            "https://www.sec.gov/Archives/edgar/data/1652044/"
            "000165204426000018/goog-20251231.htm"
        ),
    },
}

# Dell's filer-reported aggregate includes Class A, B, and C common shares.
# Those classes share dividends and undistributed earnings equally, and Class A
# and B convert into Class C one-for-one. Restrict the us-gaap Frame total to
# this reviewed issuer instead of treating every multi-class aggregate as safe.
REVIEWED_COMMON_FRAME_TOTAL_POLICIES = {
    1571996: {
        "basis": "one-to-one equal-economic Class A, B, and C common shares",
        "policy_source": (
            "https://www.sec.gov/Archives/edgar/data/1571996/"
            "000157199626000030/dell-20260501.htm"
        ),
    }
}

# Visa reports several common classes with different Class A conversion ratios.
# The combined B1/B2 member overlaps the individual members and is intentionally
# ignored. Any other class member makes the policy fail closed.
REVIEWED_CLASS_CONVERSION_POLICIES = {
    1403161: {
        "accessions": {
            "0001403161-26-000045": {
                "ratios": {
                    "CommonClassA": 1.0,
                    "CommonClassB1": 1.5475,
                    "CommonClassB2": 1.5075,
                    "CommonClassC": 4.0,
                },
                "basis": "Class A equivalent",
                "policy_source": (
                    "https://www.sec.gov/Archives/edgar/data/1403161/"
                    "000140316126000045/v-20251231.htm"
                ),
            },
            "0001403161-26-000079": {
                "ratios": {
                    "CommonClassA": 1.0,
                    "CommonClassB1": 1.5475,
                    "CommonClassB2": 1.5075,
                    "CommonClassC": 4.0,
                },
                "basis": "Class A equivalent",
                "policy_source": (
                    "https://www.sec.gov/Archives/edgar/data/1403161/"
                    "000140316126000079/v-20260331.htm"
                ),
            },
            "0001403161-26-000104": {
                "ratios": {
                    "CommonClassA": 1.0,
                    "CommonClassB1": 1.5445,
                    "CommonClassB2": 1.5014,
                    "CommonClassB3": 1.4953,
                    "CommonClassC": 4.0,
                },
                "redundant_aggregates": {},
                "basis": "Class A equivalent",
                "policy_source": (
                    "https://www.sec.gov/Archives/edgar/data/1403161/"
                    "000140316126000104/v-20260630.htm"
                ),
            },
        },
        "redundant_aggregates": {
            "CommonClassB1AndB2": ("CommonClassB1", "CommonClassB2")
        },
    }
}

# Berkshire reports weighted-average shares in both Class A- and Class
# B-equivalent units. Selecting the matching filer-reported equivalent avoids
# inventing or scraping a conversion ratio.
REPORTED_EQUIVALENT_CLASS_POLICIES = {
    1067983: {
        "BRK-A": {
            "member": "EquivalentClassA",
            "basis": "filer-reported Class A equivalent",
            "policy_source": (
                "https://www.sec.gov/Archives/edgar/data/1067983/"
                "000119312526083899/brka-20251231.htm"
            ),
        },
        "BRK-B": {
            "member": "EquivalentClassB",
            "basis": "filer-reported Class B equivalent",
            "policy_source": (
                "https://www.sec.gov/Archives/edgar/data/1067983/"
                "000119312526083899/brka-20251231.htm"
            ),
        },
    }
}
REPORTED_EQUIVALENT_COVER_MEMBERS = {
    1067983: frozenset({"CommonClassA", "CommonClassB"}),
}


@dataclass(frozen=True, order=True)
class Quarter:
    year: int
    quarter: int

    @classmethod
    def parse(cls, value: str) -> "Quarter":
        normalized = value.strip().upper()
        if len(normalized) != 6 or normalized[4] != "Q":
            raise argparse.ArgumentTypeError(
                "quarter must use YYYYQn form, for example 2025Q4"
            )
        try:
            result = cls(int(normalized[:4]), int(normalized[5]))
        except ValueError as error:
            raise argparse.ArgumentTypeError("quarter must use YYYYQn form") from error
        if result.quarter not in range(1, 5):
            raise argparse.ArgumentTypeError("quarter must be between Q1 and Q4")
        return result

    @classmethod
    def current(cls) -> "Quarter":
        today = date.today()
        return cls(today.year, (today.month - 1) // 3 + 1)

    def previous(self) -> "Quarter":
        if self.quarter == 1:
            return Quarter(self.year - 1, 4)
        return Quarter(self.year, self.quarter - 1)

    def label(self) -> str:
        return f"{self.year}Q{self.quarter}"


@dataclass(frozen=True)
class SicFact:
    sic: int
    accession: str
    filed: str
    form: str
    accelerated_filer_status: str
    source: str


@dataclass(frozen=True)
class FrameFact:
    value: int | float
    end: str
    accession: str
    frame: str
    source: str


@dataclass(frozen=True)
class ShareComponent:
    value: int | float
    end: str
    accession: str
    filed: str
    form: str
    quarters: int
    tag: str
    taxonomy: str
    segments: tuple[tuple[str, str], ...]
    source: str


@dataclass(frozen=True)
class FilingContext:
    start: str | None
    end: str
    quarters: int
    segments: tuple[tuple[str, str], ...]
    qualified_segments: tuple[tuple[str, str, str, str], ...]


@dataclass(frozen=True)
class SharesFact:
    value: int | float
    end: str
    accession: str
    filed: str
    form: str
    source: str
    method: str
    confidence: str
    components: tuple[ShareComponent, ...]
    frame: str | None = None
    basis: str | None = None
    policy_source: str | None = None
    component_multipliers: tuple[float, ...] = ()


class SecClient:
    """Small SEC-only client with a persistent cache and global rate limit."""

    def __init__(
        self, user_agent: str, requests_per_second: float, cache_dir: Path
    ) -> None:
        if not user_agent.strip():
            raise ValueError("a descriptive SEC User-Agent is required")
        if not 0 < requests_per_second <= 10:
            raise ValueError(
                "SEC request rate must be greater than zero and at most 10/s"
            )
        self.user_agent = user_agent.strip()
        self.minimum_interval = 1.0 / requests_per_second
        self.cache_dir = cache_dir.expanduser()
        self.cache_dir.mkdir(parents=True, exist_ok=True)
        self.last_request = 0.0
        self.receipts: dict[str, str] = {}

    def get(self, url: str, *, optional: bool = False) -> bytes | None:
        parsed = urllib.parse.urlparse(url)
        if parsed.scheme != "https" or parsed.hostname not in {
            "www.sec.gov",
            "data.sec.gov",
        }:
            raise ValueError(f"refusing non-SEC source: {url}")

        cache_key = hashlib.sha256(url.encode("utf-8")).hexdigest()
        suffix = Path(parsed.path).suffix or ".data"
        cache_path = self.cache_dir / f"{cache_key}{suffix}"
        metadata_path = self.cache_dir / f"{cache_key}.meta.json"
        if cache_path.is_file() and metadata_path.is_file():
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
            self.receipts[url] = str(metadata["retrieved_at"])
            return cache_path.read_bytes()

        for attempt in range(4):
            elapsed = time.monotonic() - self.last_request
            if elapsed < self.minimum_interval:
                time.sleep(self.minimum_interval - elapsed)
            request = urllib.request.Request(
                url,
                headers={
                    "User-Agent": self.user_agent,
                    "Accept": "application/json, application/zip, text/plain;q=0.9, */*;q=0.1",
                },
            )
            try:
                self.last_request = time.monotonic()
                with urllib.request.urlopen(request, timeout=90) as response:
                    payload = response.read()
                retrieved_at = utc_now()
                temporary = cache_path.with_suffix(cache_path.suffix + ".tmp")
                temporary.write_bytes(payload)
                temporary.replace(cache_path)
                metadata_path.write_text(
                    json.dumps({"url": url, "retrieved_at": retrieved_at}, indent=2)
                    + "\n",
                    encoding="utf-8",
                )
                self.receipts[url] = retrieved_at
                return payload
            except urllib.error.HTTPError as error:
                if error.code == 404 and optional:
                    return None
                if error.code not in {429, 500, 502, 503, 504} or attempt == 3:
                    raise RuntimeError(
                        f"SEC request failed ({error.code}): {url}"
                    ) from error
                retry_after = error.headers.get("Retry-After")
                delay = (
                    float(retry_after)
                    if retry_after and retry_after.isdigit()
                    else 2**attempt
                )
                time.sleep(min(delay, 30.0))
            except urllib.error.URLError as error:
                if attempt == 3:
                    raise RuntimeError(
                        f"could not reach SEC endpoint: {url}"
                    ) from error
                time.sleep(2**attempt)
        raise AssertionError("unreachable")


def utc_now() -> str:
    return (
        datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


def json_payload(payload: bytes | None, url: str) -> dict[str, Any]:
    if payload is None:
        raise RuntimeError(f"missing required SEC response: {url}")
    try:
        value = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"SEC returned invalid JSON: {url}") from error
    if not isinstance(value, dict):
        raise RuntimeError(f"SEC returned unexpected JSON shape: {url}")
    return value


def load_reviewed_share_policies(
    path: Path = SHARE_POLICY_PATH,
) -> dict[int, dict[str, Any]]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise RuntimeError(f"could not read share policy registry {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise RuntimeError(f"share policy registry {path} is invalid JSON") from error
    if not isinstance(payload, dict) or payload.get("schema_version") != 1:
        raise RuntimeError("share policy registry must use schema version 1")
    policies = payload.get("policies")
    if not isinstance(policies, list):
        raise RuntimeError("share policy registry policies must be a list")

    parsed: dict[int, dict[str, Any]] = {}
    for index, value in enumerate(policies):
        location = f"share policy {index}"
        if not isinstance(value, dict):
            raise RuntimeError(f"{location} must be an object")
        cik_text = str(value.get("cik") or "")
        if len(cik_text) != 10 or not cik_text.isdigit() or int(cik_text) <= 0:
            raise RuntimeError(f"{location} has an invalid CIK")
        cik = int(cik_text)
        if cik in parsed:
            raise RuntimeError(f"share policy registry contains duplicate CIK {cik_text}")
        symbol = str(value.get("symbol") or "")
        confidence = str(value.get("confidence") or "")
        basis = str(value.get("basis") or "")
        price_basis = str(value.get("price_basis") or "")
        policy_source = str(value.get("policy_source") or "")
        members = value.get("members")
        filing_facts = value.get("filing_facts", [])
        if not valid_ticker_symbol(symbol):
            raise RuntimeError(f"{location} has an invalid canonical symbol")
        if confidence not in {"low", "medium"}:
            raise RuntimeError(f"{location} confidence must be low or medium")
        if not basis or price_basis not in SHARE_POLICY_PRICE_BASES:
            raise RuntimeError(f"{location} must describe its basis and price basis")
        parsed_source = urllib.parse.urlparse(policy_source)
        if (
            parsed_source.scheme != "https"
            or parsed_source.hostname != "www.sec.gov"
            or not parsed_source.path.startswith("/Archives/edgar/data/")
        ):
            raise RuntimeError(
                f"{location} policy source must be an SEC filing HTTPS URL"
            )
        if not isinstance(members, dict) or not members:
            raise RuntimeError(f"{location} members must be a non-empty object")
        parsed_members: dict[str, float] = {}
        for member, multiplier in members.items():
            member_name = str(member)
            if (
                not member_name
                or not member_name.isascii()
                or len(member_name) > 128
            ):
                raise RuntimeError(f"{location} has an invalid member name")
            if (
                isinstance(multiplier, bool)
                or not isinstance(multiplier, (int, float))
                or not math.isfinite(float(multiplier))
                or float(multiplier) < 0
            ):
                raise RuntimeError(f"{location} has an invalid member multiplier")
            parsed_members[member_name] = float(multiplier)
        if not isinstance(filing_facts, list):
            raise RuntimeError(f"{location} filing_facts must be a list")
        parsed_filing_facts: list[dict[str, Any]] = []
        seen_filing_fact_selectors: set[tuple[Any, ...]] = set()
        for fact_index, fact in enumerate(filing_facts):
            fact_location = f"{location} filing fact {fact_index}"
            if not isinstance(fact, dict):
                raise RuntimeError(f"{fact_location} must be an object")
            tag = str(fact.get("tag") or "")
            accession = str(fact.get("accession") or "")
            namespace = str(fact.get("namespace") or "")
            unit = str(fact.get("unit") or "")
            quarters = fact.get("quarters")
            multiplier = fact.get("multiplier")
            segments = fact.get("segments", [])
            start = fact.get("start")
            end = str(fact.get("end") or "")
            parsed_namespace = urllib.parse.urlparse(namespace)
            if (
                not tag
                or not tag.isascii()
                or len(tag) > 160
                or not tag[0].isalpha()
                or not tag.replace("_", "").isalnum()
            ):
                raise RuntimeError(f"{fact_location} has an invalid tag")
            if (
                not valid_accession(accession)
                or parsed_namespace.scheme not in {"http", "https"}
                or not parsed_namespace.hostname
            ):
                raise RuntimeError(
                    f"{fact_location} has an invalid accession or namespace"
                )
            if unit != "shares":
                raise RuntimeError(f"{fact_location} must use shares")
            if (
                isinstance(quarters, bool)
                or not isinstance(quarters, int)
                or quarters < 0
                or quarters > 8
            ):
                raise RuntimeError(f"{fact_location} has invalid quarters")
            parsed_start = None if start is None else str(start)
            try:
                end_on = date.fromisoformat(end)
                start_on = (
                    date.fromisoformat(parsed_start)
                    if parsed_start is not None
                    else None
                )
            except ValueError as error:
                raise RuntimeError(
                    f"{fact_location} has an invalid period"
                ) from error
            if (
                (quarters == 0 and start_on is not None)
                or (quarters > 0 and start_on is None)
                or (start_on is not None and start_on > end_on)
            ):
                raise RuntimeError(f"{fact_location} has an invalid period")
            if accession.replace("-", "") not in parsed_source.path:
                raise RuntimeError(
                    f"{fact_location} accession must match the policy source"
                )
            if (
                isinstance(multiplier, bool)
                or not isinstance(multiplier, (int, float))
                or not math.isfinite(float(multiplier))
                or float(multiplier) <= 0
            ):
                raise RuntimeError(f"{fact_location} has an invalid multiplier")
            if not isinstance(segments, list):
                raise RuntimeError(f"{fact_location} segments must be a list")
            parsed_segments: list[tuple[str, str]] = []
            qualified_segments: list[tuple[str, str, str, str]] = []
            for segment in segments:
                if not isinstance(segment, dict):
                    raise RuntimeError(
                        f"{fact_location} has an invalid segment"
                    )
                axis_name = normalize_xbrl_axis(segment.get("axis"))
                member_name = normalize_xbrl_member(segment.get("member"))
                axis_namespace = str(segment.get("axis_namespace") or "")
                member_namespace = str(segment.get("member_namespace") or "")
                if (
                    axis_name is None
                    or member_name is None
                    or not valid_xbrl_namespace(axis_namespace)
                    or not valid_xbrl_namespace(member_namespace)
                ):
                    raise RuntimeError(
                        f"{fact_location} has an invalid segment"
                    )
                parsed_segments.append((axis_name, member_name))
                qualified_segments.append(
                    (
                        axis_namespace,
                        axis_name,
                        member_namespace,
                        member_name,
                    )
                )
            parsed_selector = {
                "accession": accession,
                "tag": tag,
                "namespace": namespace,
                "unit": unit,
                "quarters": quarters,
                "start": parsed_start,
                "end": end,
                "multiplier": float(multiplier),
                "segments": tuple(sorted(parsed_segments)),
                "qualified_segments": tuple(sorted(qualified_segments)),
            }
            selector_identity = filing_fact_selector_identity(parsed_selector)
            if selector_identity in seen_filing_fact_selectors:
                raise RuntimeError(
                    f"{location} contains a duplicate filing fact selector"
                )
            seen_filing_fact_selectors.add(selector_identity)
            parsed_filing_facts.append(parsed_selector)
        if not (
            any(multiplier > 0 for multiplier in parsed_members.values())
            or parsed_filing_facts
        ):
            raise RuntimeError(
                f"{location} must include at least one priced member or filing fact"
            )
        parsed[cik] = {
            "symbol": symbol,
            "confidence": confidence,
            "basis": basis,
            "price_basis": price_basis,
            "policy_source": policy_source,
            "members": parsed_members,
            "filing_facts": parsed_filing_facts,
        }
    return parsed


def quarter_sequence(start: Quarter, count: int) -> list[Quarter]:
    quarters: list[Quarter] = []
    current = start
    for _ in range(count):
        quarters.append(current)
        current = current.previous()
    return quarters


def find_latest_fsds(client: SecClient, through: Quarter) -> Quarter:
    candidate = through
    for _ in range(12):
        url = FSDS_URL.format(year=candidate.year, quarter=candidate.quarter)
        if client.get(url, optional=True) is not None:
            return candidate
        candidate = candidate.previous()
    raise RuntimeError(
        f"no SEC financial statement dataset found at or before {through.label()}"
    )


def load_tickers(client: SecClient) -> tuple[dict[int, dict[str, Any]], str]:
    response = json_payload(client.get(TICKERS_URL), TICKERS_URL)
    fields = response.get("fields")
    rows = response.get("data")
    if not isinstance(fields, list) or not isinstance(rows, list):
        raise RuntimeError("SEC ticker exchange file is missing fields or data")
    try:
        indexes = {
            name: fields.index(name) for name in ("cik", "name", "ticker", "exchange")
        }
    except ValueError as error:
        raise RuntimeError("SEC ticker exchange schema changed") from error

    grouped: dict[int, list[dict[str, Any]]] = {}
    for ordinal, row in enumerate(rows):
        if not isinstance(row, list):
            continue
        try:
            cik = int(row[indexes["cik"]])
            symbol = str(row[indexes["ticker"]] or "").strip().upper()
            name = str(row[indexes["name"]] or "").strip()
            exchange = str(row[indexes["exchange"]] or "").strip()
        except (IndexError, TypeError, ValueError):
            continue
        if (
            cik <= 0
            or not symbol
            or not name
            or exchange not in {"NYSE", "Nasdaq", "CBOE"}
            or not valid_ticker_symbol(symbol)
        ):
            continue
        grouped.setdefault(cik, []).append(
            {"symbol": symbol, "name": name, "exchange": exchange, "ordinal": ordinal}
        )

    identities = {
        cik: canonical_identity(values, cik=cik) for cik, values in grouped.items()
    }
    return identities, "sec_company_tickers_exchange"


def valid_ticker_symbol(symbol: str) -> bool:
    if not symbol or not symbol.isascii():
        return False
    segments = symbol.replace(".", "-").split("-")
    return all(
        segment
        and all("A" <= character <= "Z" or character.isdigit() for character in segment)
        for segment in segments
    )


def canonical_identity(
    identities: list[dict[str, Any]], cik: int | None = None
) -> dict[str, Any]:
    reviewed_symbol = REVIEWED_CANONICAL_SYMBOLS.get(cik)
    if reviewed_symbol is not None:
        reviewed_matches = [
            identity
            for identity in identities
            if str(identity["symbol"]) == reviewed_symbol
        ]
        if reviewed_matches:
            return min(reviewed_matches, key=lambda identity: int(identity["ordinal"]))

    sibling_symbols = {str(identity["symbol"]) for identity in identities}
    source_preferred = min(
        identities,
        key=lambda identity: (
            int(not valid_ticker_symbol(str(identity["symbol"]))),
            int(likely_derivative_symbol(identity, sibling_symbols)),
            int(identity["ordinal"]),
        ),
    )
    return min(
        identities,
        key=lambda identity: canonical_symbol_key(
            identity,
            sibling_symbols,
            str(source_preferred["symbol"]),
        ),
    )


def canonical_symbol_key(
    identity: dict[str, Any],
    sibling_symbols: set[str] | None = None,
    source_preferred_symbol: str | None = None,
) -> tuple[int, int, int, int]:
    """Prefer concise common-stock symbols, then preserve SEC file order."""
    symbol = str(identity["symbol"])
    valid_penalty = int(not valid_ticker_symbol(symbol))
    derivative_penalty = int(
        likely_derivative_symbol(identity, sibling_symbols or set())
    )
    concise_base_penalty = int(
        not concise_base_symbol(
            symbol,
            sibling_symbols or set(),
            source_preferred_symbol,
        )
    )
    return (
        valid_penalty,
        derivative_penalty,
        concise_base_penalty,
        int(identity["ordinal"]),
    )


def concise_base_symbol(
    symbol: str,
    sibling_symbols: set[str],
    source_preferred_symbol: str | None,
) -> bool:
    return (
        valid_ticker_symbol(symbol)
        and len(symbol) <= 4
        and source_preferred_symbol is not None
        and "-" not in source_preferred_symbol
        and "." not in source_preferred_symbol
        and source_preferred_symbol.startswith(symbol)
        and any(
            sibling != symbol and sibling.startswith(symbol)
            for sibling in sibling_symbols
        )
    )


def likely_derivative_symbol(
    identity: dict[str, Any], sibling_symbols: set[str]
) -> bool:
    symbol = str(identity["symbol"])
    suffix = (
        symbol.replace(".", "-").rsplit("-", 1)[-1]
        if "-" in symbol or "." in symbol
        else ""
    )
    if suffix.startswith(("P", "W", "U", "R")):
        return True

    # A sibling base symbol makes compact warrant/unit/right suffixes
    # unambiguous without penalizing common tickers such as DOW. Fifth-letter
    # codes alone are insufficient because issuers can list several preferred
    # series without a traded common base.
    return any(
        symbol.endswith(marker)
        and symbol[: -len(marker)] in sibling_symbols
        for marker in ("WS", "WT", "P", "W", "U", "R")
    )


def load_sic_facts(
    client: SecClient, quarters: Iterable[Quarter]
) -> tuple[dict[int, SicFact], list[dict[str, str]]]:
    facts: dict[int, SicFact] = {}
    sources: list[dict[str, str]] = []
    for quarter in quarters:
        url = FSDS_URL.format(year=quarter.year, quarter=quarter.quarter)
        payload = client.get(url)
        if payload is None:
            raise RuntimeError(f"missing SEC dataset {quarter.label()}")
        source_id = f"sec_fsds_{quarter.year}q{quarter.quarter}_sub"
        sources.append(source_record(source_id, url, client))
        with zipfile.ZipFile(io.BytesIO(payload)) as archive:
            member = next(
                (
                    name
                    for name in archive.namelist()
                    if name.lower().endswith("sub.txt")
                ),
                None,
            )
            if member is None:
                raise RuntimeError(f"SEC dataset {quarter.label()} has no sub.txt")
            with archive.open(member) as raw:
                reader = csv.DictReader(
                    io.TextIOWrapper(raw, encoding="utf-8-sig"), delimiter="\t"
                )
                for row in reader:
                    try:
                        cik = int(row.get("cik", ""))
                        sic = int(row.get("sic", ""))
                    except (TypeError, ValueError):
                        continue
                    filed = (row.get("filed") or "").strip()
                    accession = (row.get("adsh") or "").strip()
                    form = (row.get("form") or "").strip()
                    accelerated_filer_status = (row.get("afs") or "").strip()
                    candidate = SicFact(
                        sic,
                        accession,
                        filed,
                        form,
                        accelerated_filer_status,
                        source_id,
                    )
                    existing = facts.get(cik)
                    if existing is None or (candidate.filed, candidate.accession) > (
                        existing.filed,
                        existing.accession,
                    ):
                        facts[cik] = candidate
    return facts, sources


def load_fsds_share_facts(
    client: SecClient,
    quarters: Iterable[Quarter],
    identities: dict[int, dict[str, Any]],
    reviewed_policies: dict[int, dict[str, Any]] | None = None,
) -> tuple[
    dict[int, SharesFact],
    list[dict[str, str]],
    dict[int, list[ShareComponent]],
]:
    candidates: dict[int, list[ShareComponent]] = {}
    sources: list[dict[str, str]] = []
    for quarter in quarters:
        url = FSDS_URL.format(year=quarter.year, quarter=quarter.quarter)
        payload = client.get(url)
        if payload is None:
            raise RuntimeError(f"missing SEC dataset {quarter.label()}")
        source_id = f"sec_fsds_{quarter.year}q{quarter.quarter}_num"
        sources.append(source_record(source_id, url, client))
        with zipfile.ZipFile(io.BytesIO(payload)) as archive:
            submissions = load_eligible_submissions(archive, identities)
            member = archive_member(archive, "num.txt")
            if member is None:
                raise RuntimeError(f"SEC dataset {quarter.label()} has no num.txt")
            with archive.open(member) as raw:
                reader = csv.DictReader(
                    io.TextIOWrapper(raw, encoding="utf-8-sig"), delimiter="\t"
                )
                for row in reader:
                    submission = submissions.get((row.get("adsh") or "").strip())
                    if submission is None:
                        continue
                    component = parse_fsds_share_component(
                        row, submission, source_id
                    )
                    if component is not None:
                        candidates.setdefault(submission["cik"], []).append(component)

    facts: dict[int, SharesFact] = {}
    for cik, components in candidates.items():
        symbol = str(identities[cik]["symbol"])
        selected = select_fsds_shares_fact(
            cik,
            symbol,
            components,
            reviewed_policies,
        )
        if selected is not None:
            facts[cik] = selected
    return facts, sources, candidates


def load_latest_inline_submissions(
    client: SecClient,
    ciks: Iterable[int],
    as_of: date,
) -> tuple[dict[int, dict[str, str | int]], list[dict[str, str]]]:
    submissions: dict[int, dict[str, str | int]] = {}
    sources: list[dict[str, str]] = []
    for cik in sorted(set(ciks)):
        url = SUBMISSIONS_URL.format(cik=cik)
        payload = client.get(url, optional=True)
        if payload is None:
            continue
        source_id = f"sec_submissions_{cik:010d}"
        sources.append(source_record(source_id, url, client))
        response = json_payload(payload, url)
        filings = response.get("filings")
        recent = filings.get("recent") if isinstance(filings, dict) else None
        if not isinstance(recent, dict):
            continue
        candidate = select_latest_inline_submission(cik, recent, as_of)
        if candidate is not None:
            submissions[cik] = candidate
    return submissions, sources


def select_latest_inline_submission(
    cik: int,
    recent: dict[str, Any],
    as_of: date,
) -> dict[str, str | int] | None:
    required = (
        "accessionNumber",
        "filingDate",
        "form",
        "primaryDocument",
        "isInlineXBRL",
    )
    columns = {name: recent.get(name) for name in required}
    if any(not isinstance(values, list) for values in columns.values()):
        return None
    row_count = min(len(values) for values in columns.values())
    candidates: list[dict[str, str | int]] = []
    for index in range(row_count):
        accession = str(columns["accessionNumber"][index] or "")
        filed = str(columns["filingDate"][index] or "")
        form = str(columns["form"][index] or "").upper()
        primary_document = str(columns["primaryDocument"][index] or "")
        inline = columns["isInlineXBRL"][index]
        try:
            filed_on = date.fromisoformat(filed)
        except ValueError:
            continue
        if (
            form.removesuffix("/A") not in ELIGIBLE_FILING_FORMS
            or filed_on > as_of
            or str(inline).lower() not in {"1", "true"}
            or not valid_accession(accession)
            or not safe_filing_filename(primary_document)
        ):
            continue
        instance = extracted_instance_filename(primary_document)
        if instance is None:
            continue
        candidates.append(
            {
                "cik": cik,
                "accession": accession,
                "filed": filed,
                "form": form,
                "instance": instance,
            }
        )
    return (
        max(
            candidates,
            key=lambda candidate: (
                str(candidate["filed"]),
                str(candidate["accession"]),
            ),
        )
        if candidates
        else None
    )


def valid_accession(value: str) -> bool:
    return (
        len(value) == 20
        and value[10] == "-"
        and value[13] == "-"
        and value.replace("-", "").isdigit()
    )


def safe_filing_filename(value: str) -> bool:
    return (
        bool(value)
        and value.isascii()
        and len(value) <= 255
        and Path(value).name == value
        and "/" not in value
        and "\\" not in value
    )


def extracted_instance_filename(primary_document: str) -> str | None:
    lowered = primary_document.lower()
    for suffix in (".htm", ".html"):
        if lowered.endswith(suffix):
            return primary_document[: -len(suffix)] + "_htm.xml"
    return None


def load_filing_cover_share_components(
    client: SecClient,
    submissions: dict[int, dict[str, str | int]],
    as_of: date,
    reviewed_policies: dict[int, dict[str, Any]] | None = None,
) -> tuple[dict[int, list[ShareComponent]], list[dict[str, str]]]:
    candidates: dict[int, list[ShareComponent]] = {}
    sources: list[dict[str, str]] = []
    for cik, submission in sorted(submissions.items()):
        accession = str(submission["accession"])
        source_id = f"sec_filing_xbrl_{accession.replace('-', '')}"
        url = FILING_ARCHIVE_URL.format(
            cik=cik,
            accession=accession.replace("-", ""),
            filename=urllib.parse.quote(str(submission["instance"]), safe="._-"),
        )
        payload = client.get(url, optional=True)
        if payload is None:
            continue
        sources.append(source_record(source_id, url, client))
        components = parse_filing_cover_share_components(
            payload,
            submission,
            source_id,
            as_of,
            (reviewed_policies or {}).get(cik),
        )
        if components:
            candidates[cik] = components
    return candidates, sources


def parse_filing_cover_share_components(
    payload: bytes,
    submission: dict[str, str | int],
    source: str,
    as_of: date,
    reviewed_policy: dict[str, Any] | None = None,
) -> list[ShareComponent]:
    try:
        root = ElementTree.fromstring(payload)
        namespace_bindings: dict[str, set[str]] = {}
        for _, (prefix, namespace) in ElementTree.iterparse(
            io.BytesIO(payload),
            events=("start-ns",),
        ):
            namespace_bindings.setdefault(prefix or "", set()).add(namespace)
        namespaces = {
            prefix: next(iter(bindings))
            for prefix, bindings in namespace_bindings.items()
            if len(bindings) == 1
        }
    except ElementTree.ParseError as error:
        raise RuntimeError(
            f"SEC filing instance {submission['accession']} is invalid XML"
        ) from error

    try:
        expected_cik = int(submission["cik"])
    except (KeyError, TypeError, ValueError) as error:
        raise RuntimeError("SEC filing submission has an invalid CIK") from error
    contexts: dict[str, FilingContext] = {}
    seen_context_ids: set[str] = set()
    duplicate_context_ids: set[str] = set()
    for element in root.iter():
        if (
            xml_local_name(element.tag) != "context"
            or xml_namespace(element.tag) != XBRLI_NAMESPACE
        ):
            continue
        context_id = str(element.attrib.get("id") or "")
        if context_id in seen_context_ids:
            duplicate_context_ids.add(context_id)
            contexts.pop(context_id, None)
            continue
        seen_context_ids.add(context_id)
        instant: str | None = None
        start: str | None = None
        end: str | None = None
        segments: list[tuple[str, str]] = []
        qualified_segments: list[tuple[str, str, str, str]] = []
        invalid_dimension = False
        context_cik: int | None = None
        for child in element.iter():
            name = xml_local_name(child.tag)
            namespace = xml_namespace(child.tag)
            if name == "identifier" and namespace == XBRLI_NAMESPACE:
                identifier = (child.text or "").strip()
                scheme = str(child.attrib.get("scheme") or "")
                if (
                    scheme not in {"http://www.sec.gov/CIK", "https://www.sec.gov/CIK"}
                    or not identifier.isdigit()
                ):
                    invalid_dimension = True
                    break
                context_cik = int(identifier)
            elif name == "instant" and namespace == XBRLI_NAMESPACE:
                instant = eligible_frame_end((child.text or "").strip(), as_of)
            elif name == "startDate" and namespace == XBRLI_NAMESPACE:
                start = eligible_frame_end((child.text or "").strip(), as_of)
            elif name == "endDate" and namespace == XBRLI_NAMESPACE:
                end = eligible_frame_end((child.text or "").strip(), as_of)
            elif name == "explicitMember" and namespace == XBRLDI_NAMESPACE:
                axis_qname = resolved_qname(
                    str(child.attrib.get("dimension") or ""),
                    namespaces,
                )
                member_qname = resolved_qname(
                    (child.text or "").strip(),
                    namespaces,
                )
                if axis_qname is None or member_qname is None:
                    invalid_dimension = True
                    break
                axis_namespace, raw_axis = axis_qname
                member_namespace, raw_member = member_qname
                axis = normalize_xbrl_axis(raw_axis)
                member = normalize_xbrl_member(raw_member)
                if (
                    axis is None
                    or member is None
                    or (
                        axis == "ClassOfStock"
                        and not versioned_taxonomy_namespace(
                            axis_namespace,
                            US_GAAP_NAMESPACE_PREFIXES,
                        )
                    )
                ):
                    invalid_dimension = True
                    break
                segments.append((axis, member))
                qualified_segments.append(
                    (
                        axis_namespace,
                        axis,
                        member_namespace,
                        member,
                    )
                )
            elif name == "typedMember" and namespace == XBRLDI_NAMESPACE:
                invalid_dimension = True
                break
        period_end = instant or end
        quarters = 0 if instant else filing_duration_quarters(start, end)
        if (
            context_id
            and not invalid_dimension
            and context_cik == expected_cik
            and period_end is not None
            and quarters is not None
            and context_id not in duplicate_context_ids
        ):
            contexts[context_id] = FilingContext(
                start,
                period_end,
                quarters,
                tuple(sorted(segments)),
                tuple(sorted(qualified_segments)),
            )

    units: dict[str, str] = {}
    seen_unit_ids: set[str] = set()
    duplicate_unit_ids: set[str] = set()
    for element in root.iter():
        if (
            xml_local_name(element.tag) != "unit"
            or xml_namespace(element.tag) != XBRLI_NAMESPACE
        ):
            continue
        unit_id = str(element.attrib.get("id") or "")
        if unit_id in seen_unit_ids:
            duplicate_unit_ids.add(unit_id)
            units.pop(unit_id, None)
            continue
        seen_unit_ids.add(unit_id)
        measures = [
            resolved_measure_name((child.text or "").strip(), namespaces)
            for child in element.iter()
            if (
                xml_local_name(child.tag) == "measure"
                and xml_namespace(child.tag) == XBRLI_NAMESPACE
            )
        ]
        if (
            unit_id
            and unit_id not in duplicate_unit_ids
            and len(measures) == 1
            and measures[0] is not None
        ):
            units[unit_id] = measures[0]

    components: list[ShareComponent] = []
    filed = date.fromisoformat(str(submission["filed"])).strftime("%Y%m%d")
    for element in root.iter():
        if (
            xml_local_name(element.tag) != SHARES_TAG
            or not versioned_taxonomy_namespace(
                xml_namespace(element.tag),
                DEI_NAMESPACE_PREFIXES,
            )
            or units.get(str(element.attrib.get("unitRef") or "")) != "shares"
        ):
            continue
        context = contexts.get(str(element.attrib.get("contextRef") or ""))
        if context is None or context.quarters != 0:
            continue
        value = positive_number_from_text("".join(element.itertext()).replace(",", ""))
        if value is None:
            continue
        components.append(
            ShareComponent(
                value=value,
                end=context.end,
                accession=str(submission["accession"]),
                filed=filed,
                form=str(submission["form"]),
                quarters=0,
                tag=SHARES_TAG,
                taxonomy="dei/filing",
                segments=context.segments,
                source=source,
            )
        )
    for selector in (reviewed_policy or {}).get("filing_facts", []):
        if str(submission["accession"]) != selector["accession"]:
            continue
        for element in root.iter():
            namespace = xml_namespace(element.tag)
            if (
                xml_local_name(element.tag) != selector["tag"]
                or namespace != selector["namespace"]
                or units.get(str(element.attrib.get("unitRef") or ""))
                != selector["unit"]
            ):
                continue
            context = contexts.get(str(element.attrib.get("contextRef") or ""))
            if (
                context is None
                or context.quarters != selector["quarters"]
                or context.start != selector["start"]
                or context.end != selector["end"]
                or context.segments != selector["segments"]
                or context.qualified_segments
                != selector["qualified_segments"]
            ):
                continue
            value = positive_number_from_text(
                "".join(element.itertext()).replace(",", "")
            )
            if value is None:
                continue
            components.append(
                ShareComponent(
                    value=value,
                    end=context.end,
                    accession=str(submission["accession"]),
                    filed=filed,
                    form=str(submission["form"]),
                    quarters=context.quarters,
                    tag=str(selector["tag"]),
                    taxonomy=f"{namespace}#filing",
                    segments=context.segments,
                    source=source,
                )
            )
    return components


def xml_local_name(value: str) -> str:
    return value.rsplit("}", 1)[-1]


def xml_namespace(value: str) -> str:
    return value[1:].split("}", 1)[0] if value.startswith("{") else ""


def normalize_xbrl_axis(value: Any) -> str | None:
    normalized = str(value or "").rsplit(":", 1)[-1]
    if normalized.endswith("Axis"):
        normalized = normalized[:-4]
    if normalized == "StatementClassOfStock":
        normalized = "ClassOfStock"
    return normalized or None


def normalize_xbrl_member(value: Any) -> str | None:
    normalized = str(value or "").strip().rsplit(":", 1)[-1]
    if normalized.endswith("Member"):
        normalized = normalized[:-6]
    return normalized or None


def valid_xbrl_namespace(value: str) -> bool:
    parsed = urllib.parse.urlparse(value)
    return parsed.scheme in {"http", "https"} and bool(parsed.hostname)


def versioned_taxonomy_namespace(
    value: str,
    prefixes: tuple[str, ...],
) -> bool:
    return any(
        len(version := value.removeprefix(prefix)) == 4
        and version.isdigit()
        for prefix in prefixes
        if value.startswith(prefix)
    )


def filing_fact_selector_identity(
    selector: dict[str, Any],
) -> tuple[Any, ...]:
    return (
        selector["accession"],
        selector["tag"],
        selector["namespace"],
        selector["unit"],
        selector["quarters"],
        selector["start"],
        selector["end"],
        selector["segments"],
        selector["qualified_segments"],
    )


def resolved_qname(
    value: str,
    namespaces: dict[str, str],
) -> tuple[str, str] | None:
    prefix, separator, local_name = value.partition(":")
    if separator:
        namespace = namespaces.get(prefix)
    else:
        local_name = prefix
        namespace = namespaces.get("")
    return (namespace, local_name) if namespace and local_name else None


def resolved_measure_name(
    value: str,
    namespaces: dict[str, str],
) -> str | None:
    resolved = resolved_qname(value, namespaces)
    if resolved is None:
        return None
    namespace, local_name = resolved
    return (
        local_name
        if namespace == XBRLI_NAMESPACE and local_name == "shares"
        else None
    )


def filing_duration_quarters(
    start: str | None,
    end: str | None,
) -> int | None:
    if start is None or end is None:
        return None
    try:
        start_on = date.fromisoformat(start)
        end_on = date.fromisoformat(end)
    except ValueError:
        return None
    days = (end_on - start_on).days + 1
    if days <= 0:
        return None
    quarters = max(1, round(days / 91.3125))
    return quarters if quarters <= 8 else None


def archive_member(archive: zipfile.ZipFile, filename: str) -> str | None:
    normalized = filename.lower()
    return next(
        (name for name in archive.namelist() if name.lower().endswith(normalized)),
        None,
    )


def load_eligible_submissions(
    archive: zipfile.ZipFile, identities: dict[int, dict[str, Any]]
) -> dict[str, dict[str, Any]]:
    member = archive_member(archive, "sub.txt")
    if member is None:
        raise RuntimeError("SEC financial statement dataset has no sub.txt")
    submissions: dict[str, dict[str, Any]] = {}
    with archive.open(member) as raw:
        reader = csv.DictReader(
            io.TextIOWrapper(raw, encoding="utf-8-sig"), delimiter="\t"
        )
        for row in reader:
            try:
                cik = int(row.get("cik", ""))
            except (TypeError, ValueError):
                continue
            form = (row.get("form") or "").strip().upper()
            base_form = form.removesuffix("/A")
            accession = (row.get("adsh") or "").strip()
            if (
                cik not in identities
                or base_form not in ELIGIBLE_FILING_FORMS
                or not accession
            ):
                continue
            submissions[accession] = {
                "cik": cik,
                "accession": accession,
                "filed": (row.get("filed") or "").strip(),
                "form": form,
            }
    return submissions


def parse_fsds_share_component(
    row: dict[str, Any], submission: dict[str, Any], source: str
) -> ShareComponent | None:
    tag = str(row.get("tag") or "").strip()
    if tag not in {
        SHARES_TAG,
        COMMON_SHARES_TAG,
        BASIC_WEIGHTED_SHARES_TAG,
        LIMITED_PARTNERS_WEIGHTED_UNITS_TAG,
    }:
        return None
    taxonomy = str(row.get("version") or "").strip()
    expected_taxonomy = "dei/" if tag == SHARES_TAG else "us-gaap/"
    if not taxonomy.startswith(expected_taxonomy):
        return None
    if (row.get("uom") or "").strip().lower() != "shares":
        return None
    if (row.get("coreg") or "").strip():
        return None
    try:
        quarters = int(row.get("qtrs") or 0)
    except (TypeError, ValueError):
        return None
    duration_tags = {
        BASIC_WEIGHTED_SHARES_TAG,
        LIMITED_PARTNERS_WEIGHTED_UNITS_TAG,
    }
    if tag not in duration_tags and quarters != 0:
        return None
    if tag in duration_tags and quarters <= 0:
        return None
    value = positive_number_from_text(row.get("value"))
    end = compact_date(row.get("ddate"))
    segments = parse_segments(row.get("segments"))
    if value is None or end is None or segments is None:
        return None
    return ShareComponent(
        value=value,
        end=end,
        accession=str(submission["accession"]),
        filed=str(submission["filed"]),
        form=str(submission["form"]),
        quarters=quarters,
        tag=tag,
        taxonomy=taxonomy,
        segments=segments,
        source=source,
    )


def positive_number_from_text(value: Any) -> int | float | None:
    try:
        parsed = float(str(value).strip())
    except (TypeError, ValueError):
        return None
    if not math.isfinite(parsed) or parsed <= 0:
        return None
    return int(parsed) if parsed.is_integer() else parsed


def compact_date(value: Any) -> str | None:
    normalized = str(value or "").strip()
    if len(normalized) != 8 or not normalized.isdigit():
        return None
    try:
        parsed = date.fromisoformat(
            f"{normalized[:4]}-{normalized[4:6]}-{normalized[6:]}"
        )
    except ValueError:
        return None
    return parsed.isoformat()


def parse_segments(value: Any) -> tuple[tuple[str, str], ...] | None:
    normalized = str(value or "").strip()
    if not normalized:
        return ()
    parsed: list[tuple[str, str]] = []
    for item in normalized.split(";"):
        item = item.strip()
        if not item:
            continue
        axis, separator, member = item.partition("=")
        if not separator or not axis.strip() or not member.strip():
            return None
        parsed.append((axis.strip(), member.strip()))
    return tuple(sorted(parsed))


def select_fsds_shares_fact(
    cik: int,
    symbol: str,
    components: list[ShareComponent],
    reviewed_policies: dict[int, dict[str, Any]] | None = None,
) -> SharesFact | None:
    reviewed_policies = reviewed_policies or {}
    if cik in reviewed_policies:
        return select_reviewed_filing_policy(
            cik,
            symbol,
            components,
            reviewed_policies[cik],
        )
    cover = latest_filing_cover_components(components)
    if cik in REPORTED_EQUIVALENT_CLASS_POLICIES:
        if cover and filing_cover_member_signature(cover) != (
            REPORTED_EQUIVALENT_COVER_MEMBERS[cik]
        ):
            return None
        components = [
            component
            for component in components
            if component.taxonomy != "dei/filing"
        ]
    if reviewed_multiclass_issuer(cik, reviewed_policies):
        components = latest_reviewed_timeline_components(components)
        if not components:
            return None
    elif cover:
        cover_facts = [
            fact
            for fact in (
                select_issuer_total(cik, cover, SHARES_TAG),
                select_single_class_filing_cover(cover),
            )
            if fact is not None
        ]
        return select_preferred_shares_fact(cover_facts)
    groups: dict[tuple[str, str, str], list[ShareComponent]] = {}
    for component in components:
        groups.setdefault(
            (component.accession, component.end, component.tag), []
        ).append(component)

    strategies: tuple[Any, ...] = (
        lambda values: select_reviewed_class_conversion(cik, values, SHARES_TAG),
        lambda values: select_issuer_total(cik, values, SHARES_TAG),
        lambda values: select_reviewed_class_sum(cik, values, SHARES_TAG),
        lambda values: select_reviewed_class_conversion(
            cik, values, COMMON_SHARES_TAG
        ),
        lambda values: select_issuer_total(cik, values, COMMON_SHARES_TAG),
        lambda values: select_reviewed_class_sum(cik, values, COMMON_SHARES_TAG),
        lambda values: select_reported_equivalent(cik, symbol, values),
    )
    if not reviewed_multiclass_issuer(cik, reviewed_policies):
        strategies += (
            select_single_class_filing_cover,
            select_basic_weighted_total,
            select_limited_partners_weighted_total,
        )
    facts: list[SharesFact] = []
    for strategy in strategies:
        facts.extend(
            fact
            for values in groups.values()
            if (fact := strategy(values)) is not None
        )
    return select_preferred_shares_fact(facts)


def select_reviewed_filing_policy(
    cik: int,
    symbol: str,
    components: list[ShareComponent],
    policy: dict[str, Any],
) -> SharesFact | None:
    if symbol != policy["symbol"]:
        return None
    cover = latest_filing_cover_components(components)
    if not cover:
        return None
    by_member: dict[str, list[ShareComponent]] = {}
    unsegmented: list[ShareComponent] = []
    for component in cover:
        if not component.segments:
            unsegmented.append(component)
            continue
        member = exact_class_member(component)
        if component.segments and member is None:
            return None
        if member is not None:
            by_member.setdefault(member, []).append(component)
    if unsegmented and select_consistent_component(unsegmented) is None:
        return None
    expected_members = policy["members"]
    if frozenset(by_member) != frozenset(expected_members):
        return None

    selected: list[ShareComponent] = []
    multipliers: list[float] = []
    total = 0.0
    for member, multiplier in sorted(expected_members.items()):
        component = select_consistent_component(by_member[member])
        if component is None:
            return None
        selected.append(component)
        multipliers.append(float(multiplier))
        total += float(component.value) * float(multiplier)
    filing_facts = policy.get("filing_facts", [])
    if len(
        {
            filing_fact_selector_identity(selector)
            for selector in filing_facts
        }
    ) != len(filing_facts):
        return None
    for selector in filing_facts:
        eligible = [
            component
            for component in components
            if (
                component.accession == cover[0].accession
                and component.accession == selector["accession"]
                and component.tag == selector["tag"]
                and component.taxonomy == f"{selector['namespace']}#filing"
                and component.quarters == selector["quarters"]
                and component.end == selector["end"]
                and component.segments == selector["segments"]
            )
        ]
        component = select_consistent_component(eligible)
        if component is None:
            return None
        multiplier = float(selector["multiplier"])
        selected.append(component)
        multipliers.append(multiplier)
        total += float(component.value) * multiplier
    if not math.isfinite(total) or total <= 0:
        return None
    first = min(
        (
            component
            for component, multiplier in zip(selected, multipliers)
            if multiplier > 0
        ),
        key=lambda component: component.end,
    )
    value: int | float = int(total) if total.is_integer() else total
    return SharesFact(
        value=value,
        end=first.end,
        accession=first.accession,
        filed=first.filed,
        form=first.form,
        source=first.source,
        method=(
            "filing_reviewed_fact_policy"
            if policy.get("filing_facts")
            else "filing_cover_reviewed_policy"
        ),
        confidence=str(policy["confidence"]),
        components=tuple(selected),
        basis=f"{policy['basis']}; {policy['price_basis']}",
        policy_source=str(policy["policy_source"]),
        component_multipliers=tuple(multipliers),
    )


def select_single_class_filing_cover(
    components: list[ShareComponent],
) -> SharesFact | None:
    cover = latest_filing_cover_components(components)
    if not cover:
        return None
    by_member: dict[str, list[ShareComponent]] = {}
    unsegmented: list[ShareComponent] = []
    for component in cover:
        if not component.segments:
            unsegmented.append(component)
            continue
        member = exact_class_member(component)
        if member is None:
            return None
        if excluded_cover_member(member):
            continue
        by_member.setdefault(member, []).append(component)

    if unsegmented:
        component = select_consistent_component(unsegmented)
        return (
            shares_fact((component,), "filing_cover_single_class", "high")
            if component is not None and len(by_member) <= 1
            else None
        )
    if len(by_member) != 1:
        return None
    component = select_consistent_component(next(iter(by_member.values())))
    return (
        shares_fact((component,), "filing_cover_single_class", "high")
        if component is not None
        else None
    )


def latest_filing_cover_components(
    components: list[ShareComponent],
) -> list[ShareComponent]:
    cover = [
        component
        for component in components
        if component.tag == SHARES_TAG
        and component.taxonomy == "dei/filing"
        and component.source.startswith("sec_filing_xbrl_")
    ]
    if not cover:
        return []
    latest_filing = max(
        (component.filed, component.accession) for component in cover
    )
    filing = [
        component
        for component in cover
        if (component.filed, component.accession) == latest_filing
    ]
    latest_end = max(component.end for component in filing)
    return [component for component in filing if component.end == latest_end]


def exact_class_member(component: ShareComponent) -> str | None:
    return (
        component.segments[0][1]
        if len(component.segments) == 1
        and component.segments[0][0] == "ClassOfStock"
        else None
    )


def filing_cover_member_signature(
    components: list[ShareComponent],
) -> frozenset[str] | None:
    by_member: dict[str, list[ShareComponent]] = {}
    unsegmented: list[ShareComponent] = []
    for component in components:
        if not component.segments:
            unsegmented.append(component)
            continue
        member = exact_class_member(component)
        if member is None:
            return None
        by_member.setdefault(member, []).append(component)
    if any(
        select_consistent_component(values) is None
        for values in by_member.values()
    ) or (
        unsegmented
        and select_consistent_component(unsegmented) is None
    ):
        return None
    return frozenset(by_member)


def excluded_cover_member(member: str) -> bool:
    normalized = member.lower()
    return any(
        marker in normalized
        for marker in (
            "preferred",
            "warrant",
            "right",
            "option",
            "debt",
            "redeemable",
            "temporary",
        )
    )


def select_consistent_component(
    components: list[ShareComponent],
) -> ShareComponent | None:
    if not components:
        return None
    selected = select_least_dimensioned(components)
    if selected is None:
        return None
    return (
        selected
        if all(
            math.isclose(
                float(component.value),
                float(selected.value),
                rel_tol=0.0001,
                abs_tol=1.0,
            )
            for component in components
        )
        else None
    )


def latest_reviewed_timeline_components(
    components: list[ShareComponent],
) -> list[ShareComponent]:
    if not components:
        return []
    newest_filing = max(
        (component.filed, component.accession) for component in components
    )
    filing_components = [
        component
        for component in components
        if (component.filed, component.accession) == newest_filing
    ]
    newest_end = max(component.end for component in filing_components)
    return [
        component for component in filing_components if component.end == newest_end
    ]


def select_issuer_total(
    cik: int, components: list[ShareComponent], tag: str
) -> SharesFact | None:
    if (
        cik in REVIEWED_CLASS_CONVERSION_POLICIES
        or cik in REPORTED_EQUIVALENT_CLASS_POLICIES
        or cik in REVIEWED_COMMON_FRAME_TOTAL_POLICIES
    ):
        return None
    eligible = [
        component
        for component in components
        if component.tag == tag
        and (
            not component.segments
            if tag == SHARES_TAG
            else common_aggregate_segments(component.segments)
        )
    ]
    component = (
        select_consistent_component(eligible)
        if any(candidate.taxonomy == "dei/filing" for candidate in eligible)
        else select_least_dimensioned(eligible)
    )
    if component is None:
        return None
    if cik in REVIEWED_EQUAL_CLASS_MEMBERS:
        reviewed_sum = select_reviewed_class_sum(cik, components, tag)
        if reviewed_sum is None or not math.isclose(
            float(component.value),
            float(reviewed_sum.value),
            rel_tol=0.0001,
            abs_tol=1.0,
        ):
            return None
    method = (
        "fsds_dei_cover_total"
        if tag == SHARES_TAG
        else "fsds_common_stock_total"
    )
    confidence = "high" if tag == SHARES_TAG else "medium"
    return shares_fact((component,), method, confidence)


def common_aggregate_segments(segments: tuple[tuple[str, str], ...]) -> bool:
    if not segments:
        return True
    return all(
        axis == "EquityComponents"
        and "commonstock" in member.lower()
        and "preferred" not in member.lower()
        for axis, member in segments
    )


def reviewed_class_components(
    components: list[ShareComponent],
) -> tuple[dict[str, list[ShareComponent]], bool] | None:
    strict = any(component.taxonomy == "dei/filing" for component in components)
    by_member: dict[str, list[ShareComponent]] = {}
    unsegmented: list[ShareComponent] = []
    for component in components:
        if not component.segments:
            unsegmented.append(component)
            continue
        member = (
            exact_class_member(component)
            if strict
            else segment_member(component.segments, "ClassOfStock")
        )
        if strict and member is None:
            return None
        if member is not None:
            by_member.setdefault(member, []).append(component)
    if (
        strict
        and unsegmented
        and select_consistent_component(unsegmented) is None
    ):
        return None
    return by_member, strict


def select_reviewed_component(
    components: list[ShareComponent],
    strict: bool,
) -> ShareComponent | None:
    return (
        select_consistent_component(components)
        if strict
        else select_least_dimensioned(components)
    )


def select_reviewed_class_sum(
    cik: int, components: list[ShareComponent], tag: str
) -> SharesFact | None:
    reviewed_members = REVIEWED_EQUAL_CLASS_MEMBERS.get(cik)
    if not reviewed_members or not components or components[0].tag != tag:
        return None
    resolved = reviewed_class_components(components)
    if resolved is None:
        return None
    by_member, strict = resolved
    if frozenset(by_member) != reviewed_members:
        return None
    selected_components: list[ShareComponent] = []
    for member in sorted(reviewed_members):
        component = select_reviewed_component(by_member[member], strict)
        if component is None:
            return None
        selected_components.append(component)
    selected = tuple(selected_components)
    aggregates = [
        component
        for component in components
        if (
            not component.segments
            if tag == SHARES_TAG
            else common_aggregate_segments(component.segments)
        )
    ]
    aggregate = select_reviewed_component(aggregates, strict)
    if aggregates and aggregate is None:
        return None
    selected_total = sum(float(component.value) for component in selected)
    if aggregate is not None and not math.isclose(
        float(aggregate.value),
        selected_total,
        rel_tol=0.0001,
        abs_tol=1.0,
    ):
        return None
    method = (
        "fsds_dei_reviewed_class_sum"
        if tag == SHARES_TAG
        else "fsds_reviewed_equal_class_sum"
    )
    confidence = "high" if tag == SHARES_TAG else "medium"
    metadata = REVIEWED_EQUAL_CLASS_POLICY_METADATA[cik]
    return shares_fact(
        selected,
        method,
        confidence,
        basis=metadata["basis"],
        policy_source=metadata["policy_source"],
        component_multipliers=tuple(1.0 for _ in selected),
    )


def select_reviewed_class_conversion(
    cik: int, components: list[ShareComponent], tag: str
) -> SharesFact | None:
    policy = REVIEWED_CLASS_CONVERSION_POLICIES.get(cik)
    if policy is None or not components or components[0].tag != tag:
        return None
    policy_version = policy["accessions"].get(components[0].accession)
    if policy_version is None:
        return None
    ratios = policy_version["ratios"]
    redundant_aggregates = policy_version.get(
        "redundant_aggregates",
        policy["redundant_aggregates"],
    )
    resolved = reviewed_class_components(components)
    if resolved is None:
        return None
    by_member, strict = resolved
    observed_members = frozenset(by_member)
    if observed_members - frozenset(ratios) - frozenset(redundant_aggregates):
        return None
    if not frozenset(ratios).issubset(observed_members):
        return None
    for aggregate_member, constituent_members in redundant_aggregates.items():
        aggregate = select_reviewed_component(
            by_member.get(aggregate_member, []),
            strict,
        )
        if aggregate is None:
            return None
        constituents = [
            select_reviewed_component(by_member.get(member, []), strict)
            for member in constituent_members
        ]
        if any(component is None for component in constituents):
            return None
        constituent_total = sum(
            float(component.value)
            for component in constituents
            if component is not None
        )
        if not math.isclose(
            float(aggregate.value),
            constituent_total,
            rel_tol=0.001,
            abs_tol=1.0,
        ):
            return None

    selected: list[ShareComponent] = []
    multipliers: list[float] = []
    total = 0.0
    for member, ratio in sorted(ratios.items()):
        component = select_reviewed_component(by_member[member], strict)
        if component is None:
            return None
        selected.append(component)
        multipliers.append(float(ratio))
        total += float(component.value) * float(ratio)
    value: int | float = int(total) if total.is_integer() else total
    first = selected[0]
    return SharesFact(
        value=value,
        end=first.end,
        accession=first.accession,
        filed=first.filed,
        form=first.form,
        source=first.source,
        method="fsds_reviewed_class_conversion",
        confidence="medium",
        components=tuple(selected),
        basis=str(policy_version["basis"]),
        policy_source=str(policy_version["policy_source"]),
        component_multipliers=tuple(multipliers),
    )


def select_reported_equivalent(
    cik: int, symbol: str, components: list[ShareComponent]
) -> SharesFact | None:
    policy = REPORTED_EQUIVALENT_CLASS_POLICIES.get(cik, {}).get(symbol)
    if policy is None:
        return None
    eligible = [
        component
        for component in components
        if component.tag == BASIC_WEIGHTED_SHARES_TAG
        and segment_member(component.segments, "ClassOfStock") == policy["member"]
    ]
    component = select_least_dimensioned(eligible)
    return (
        shares_fact(
            (component,),
            "fsds_reported_equivalent_class",
            "low",
            basis=str(policy["basis"]),
            policy_source=str(policy["policy_source"]),
            component_multipliers=(1.0,),
        )
        if component
        else None
    )


def select_basic_weighted_total(
    components: list[ShareComponent],
) -> SharesFact | None:
    eligible = [
        component
        for component in components
        if component.tag == BASIC_WEIGHTED_SHARES_TAG and not component.segments
    ]
    component = select_least_dimensioned(eligible)
    return (
        shares_fact((component,), "fsds_basic_weighted_average", "low")
        if component
        else None
    )


def select_limited_partners_weighted_total(
    components: list[ShareComponent],
) -> SharesFact | None:
    eligible = [
        component
        for component in components
        if component.tag == LIMITED_PARTNERS_WEIGHTED_UNITS_TAG
        and (
            not component.segments
            or (
                len(component.segments) == 1
                and component.segments[0]
                in {
                    ("ClassOfStock", "CommonUnits"),
                    ("EquityComponents", "CommonUnits"),
                    ("LimitedPartnersCapitalAccountByClass", "CommonUnits"),
                }
            )
        )
    ]
    by_shape: dict[tuple[tuple[str, str], ...], list[ShareComponent]] = {}
    for component in eligible:
        by_shape.setdefault(component.segments, []).append(component)
    selected_by_shape = [
        selected
        for values in by_shape.values()
        if (selected := select_least_dimensioned(values)) is not None
    ]
    if not selected_by_shape:
        return None
    values = {round(float(component.value), 6) for component in selected_by_shape}
    if len(values) != 1:
        return None
    component = min(
        selected_by_shape,
        key=lambda value: (len(value.segments), value.segments),
    )
    return shares_fact(
        (component,),
        "fsds_limited_partners_weighted_average",
        "low",
    )


def segment_member(
    segments: tuple[tuple[str, str], ...], expected_axis: str
) -> str | None:
    members = [member for axis, member in segments if axis == expected_axis]
    return members[0] if len(members) == 1 else None


def select_least_dimensioned(
    components: list[ShareComponent],
) -> ShareComponent | None:
    return (
        min(
            components,
            key=lambda component: (
                duration_penalty(component),
                len(component.segments),
                component.segments,
                component.taxonomy,
            ),
        )
        if components
        else None
    )


def duration_penalty(component: ShareComponent) -> tuple[int, int]:
    base_form = component.form.upper().removesuffix("/A")
    expected = 4 if base_form in {"10-K", "20-F", "40-F"} else 1
    return abs(component.quarters - expected), component.quarters


def reviewed_multiclass_issuer(
    cik: int,
    reviewed_policies: dict[int, dict[str, Any]] | None = None,
) -> bool:
    return (
        cik in (reviewed_policies or {})
        or cik in REVIEWED_EQUAL_CLASS_MEMBERS
        or cik in REVIEWED_CLASS_CONVERSION_POLICIES
        or cik in REPORTED_EQUIVALENT_CLASS_POLICIES
        or cik in REVIEWED_COMMON_FRAME_TOTAL_POLICIES
    )


def shares_fact(
    components: tuple[ShareComponent, ...],
    method: str,
    confidence: str,
    *,
    basis: str | None = None,
    policy_source: str | None = None,
    component_multipliers: tuple[float, ...] = (),
) -> SharesFact:
    first = components[0]
    total = sum(float(component.value) for component in components)
    value: int | float = int(total) if total.is_integer() else total
    return SharesFact(
        value=value,
        end=first.end,
        accession=first.accession,
        filed=first.filed,
        form=first.form,
        source=first.source,
        method=method,
        confidence=confidence,
        components=components,
        basis=basis,
        policy_source=policy_source,
        component_multipliers=component_multipliers,
    )


def merge_share_facts(
    frame_facts: dict[int, FrameFact],
    fsds_facts: dict[int, SharesFact],
    common_frame_facts: dict[int, FrameFact] | None = None,
    reviewed_policies: dict[int, dict[str, Any]] | None = None,
    as_of: date | None = None,
) -> dict[int, SharesFact]:
    reviewed_policies = reviewed_policies or {}
    candidates: dict[int, list[SharesFact]] = {
        cik: [fact] for cik, fact in fsds_facts.items()
    }
    for cik, frame_fact in frame_facts.items():
        if reviewed_multiclass_issuer(cik, reviewed_policies):
            continue
        candidates.setdefault(cik, []).append(
            SharesFact(
                value=frame_fact.value,
                end=frame_fact.end,
                accession=frame_fact.accession,
                filed="",
                form="",
                source=frame_fact.source,
                method="sec_frame_dei_total",
                confidence="high",
                components=(),
                frame=frame_fact.frame,
            )
        )
    for cik, frame_fact in (common_frame_facts or {}).items():
        if cik in reviewed_policies:
            continue
        policy = REVIEWED_COMMON_FRAME_TOTAL_POLICIES.get(cik)
        if policy is None:
            continue
        candidates.setdefault(cik, []).append(
            SharesFact(
                value=frame_fact.value,
                end=frame_fact.end,
                accession=frame_fact.accession,
                filed="",
                form="",
                source=frame_fact.source,
                method="sec_frame_reviewed_common_total",
                confidence="medium",
                components=(),
                frame=frame_fact.frame,
                basis=str(policy["basis"]),
                policy_source=str(policy["policy_source"]),
            )
        )
    return {
        cik: selected
        for cik, facts in candidates.items()
        if (
            selected := select_preferred_shares_fact(
                facts,
                as_of=as_of,
            )
        )
        is not None
    }


def select_preferred_shares_fact(
    facts: list[SharesFact],
    *,
    as_of: date | None = None,
) -> SharesFact | None:
    if as_of is not None:
        facts = [
            fact
            for fact in facts
            if 0
            <= (as_of - date.fromisoformat(fact.end)).days
            <= MAX_SHARE_FACT_AGE_DAYS
        ]
    if not facts:
        return None
    newest_date = max(date.fromisoformat(fact.end) for fact in facts)
    newest_facts = [
        fact for fact in facts if date.fromisoformat(fact.end) == newest_date
    ]
    newest_confidence = max(
        {"low": 0, "medium": 1, "high": 2}[fact.confidence]
        for fact in newest_facts
    )
    override_days = (
        MAX_WEIGHTED_FALLBACK_OVERRIDE_DAYS
        if newest_confidence == 0
        else MAX_POINT_FACT_OVERRIDE_DAYS
    )
    fresh = [
        fact
        for fact in facts
        if (
            newest_date - date.fromisoformat(fact.end)
        ).days
        <= override_days
    ]
    confidence_rank = {"low": 0, "medium": 1, "high": 2}
    method_rank = {
        "filing_reviewed_fact_policy": 7,
        "filing_cover_reviewed_policy": 7,
        "filing_cover_single_class": 6,
        "fsds_reviewed_class_conversion": 6,
        "fsds_dei_cover_total": 5,
        "fsds_dei_reviewed_class_sum": 4,
        "sec_frame_dei_total": 3,
        "sec_frame_reviewed_common_total": 3,
        "fsds_reviewed_equal_class_sum": 3,
        "fsds_common_stock_total": 2,
        "fsds_reported_equivalent_class": 0,
        "fsds_basic_weighted_average": 0,
        "fsds_limited_partners_weighted_average": 0,
    }
    return max(
        fresh,
        key=lambda fact: (
            confidence_rank[fact.confidence],
            fact.end,
            method_rank[fact.method],
            fact.filed,
            fact.accession,
        ),
    )


def load_frame_facts(
    client: SecClient,
    quarters: Iterable[Quarter],
    tag: str,
    unit: str,
    as_of: date,
    *,
    taxonomy: str = "dei",
) -> tuple[dict[int, FrameFact], list[dict[str, str]]]:
    if taxonomy not in {"dei", "us-gaap"}:
        raise ValueError(f"unsupported SEC Frame taxonomy: {taxonomy}")
    candidates: dict[int, list[FrameFact]] = {}
    sources: list[dict[str, str]] = []
    for quarter in quarters:
        frame = f"CY{quarter.year}Q{quarter.quarter}I"
        url = FRAME_URL.format(
            taxonomy=taxonomy,
            tag=tag,
            unit=unit,
            year=quarter.year,
            quarter=quarter.quarter,
        )
        payload = client.get(url, optional=True)
        if payload is None:
            continue
        taxonomy_prefix = "" if taxonomy == "dei" else f"{snake_case(taxonomy)}_"
        source_id = f"sec_frame_{taxonomy_prefix}{snake_case(tag)}_{frame}"
        sources.append(source_record(source_id, url, client))
        response = json_payload(payload, url)
        data = response.get("data")
        if not isinstance(data, list):
            raise RuntimeError(f"SEC frame {frame} is missing data")
        for row in data:
            if not isinstance(row, dict):
                continue
            try:
                cik = int(row["cik"])
            except (KeyError, TypeError, ValueError):
                continue
            value = positive_number(row.get("val"))
            end = eligible_frame_end(row.get("end"), as_of)
            if value is None or end is None:
                continue
            candidate = FrameFact(
                value=value,
                end=end,
                accession=str(row.get("accn") or ""),
                frame=frame,
                source=source_id,
            )
            candidates.setdefault(cik, []).append(candidate)
    facts = {
        cik: select_frame_fact(values, screen_temporal_outlier=tag == PUBLIC_FLOAT_TAG)
        for cik, values in candidates.items()
    }
    return facts, sources


def eligible_frame_end(value: Any, as_of: date) -> str | None:
    end = str(value or "")
    try:
        parsed = date.fromisoformat(end)
    except ValueError:
        return None
    return end if parsed <= as_of else None


def select_frame_fact(
    facts: list[FrameFact], *, screen_temporal_outlier: bool
) -> FrameFact:
    ordered = sorted(
        facts,
        key=lambda fact: (fact.end, fact.frame, fact.accession),
        reverse=True,
    )
    newest = ordered[0]
    if screen_temporal_outlier and len(ordered) >= 2:
        older_values = sorted(float(fact.value) for fact in ordered[1:])
        median = older_values[len(older_values) // 2]
        ratio = float(newest.value) / median
        if ratio > 100:
            return ordered[1]
    return newest


def positive_number(value: Any) -> int | float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if not math.isfinite(float(value)) or value <= 0:
        return None
    return value


def snake_case(value: str) -> str:
    output: list[str] = []
    for index, character in enumerate(value):
        if character.isupper() and index:
            output.append("_")
        output.append(character.lower())
    return "".join(output)


def source_record(source_id: str, url: str, client: SecClient) -> dict[str, str]:
    return {"id": source_id, "url": url, "retrieved_at": client.receipts[url]}


def sector_for_sic(sic: int) -> str:
    """Map SEC SIC codes into StockTouch's nine legacy display sectors."""
    if sic in {1220, 1221, 1311, 1321, 1381, 1382, 1389, 2911, 4612, 4613, 5171}:
        return "energy"
    if 4900 <= sic <= 4999 and sic not in {4953}:
        return "utilities"
    if 6000 <= sic <= 6799:
        return "financial"
    if 8000 <= sic <= 8099 or 2830 <= sic <= 2836 or 3841 <= sic <= 3851:
        return "healthcare"
    if sic in {5047, 5122, 5912}:
        return "healthcare"
    if (
        3570 <= sic <= 3579
        or 3660 <= sic <= 3679
        or sic in {3695, 3823, 3825, 3826, 3827, 3829, 5045, 5065}
        or 7370 <= sic <= 7379
    ):
        return "technology"
    if sic in {
        2840,
        2841,
        2842,
        2843,
        2844,
        3011,
        3021,
        3711,
        3714,
        3751,
        3911,
        3914,
        3931,
        3942,
        3944,
    }:
        return "consumer"
    if 1000 <= sic <= 1299 or 1400 <= sic <= 1499:
        return "materials"
    if 800 <= sic <= 899 or 2400 <= sic <= 2699 or 3200 <= sic <= 3399:
        return "materials"
    if 2800 <= sic <= 2899 or 3000 <= sic <= 3099:
        return "materials"
    if 100 <= sic <= 799 or 2000 <= sic <= 2399 or 2500 <= sic <= 2599:
        return "consumer"
    if 3900 <= sic <= 3999:
        return "consumer"
    if 4700 <= sic <= 4729:
        return "services"
    if 1500 <= sic <= 1799 or 3400 <= sic <= 3799 or 4000 <= sic <= 4799:
        return "industrial"
    if sic == 4953:
        return "industrial"
    if 5000 <= sic <= 5999 or 7000 <= sic <= 7999 or 8100 <= sic <= 9999:
        return "services"
    if 2700 <= sic <= 2799 or 4800 <= sic <= 4899:
        return "services"
    return "industrial"


def fact_provenance(fact: FrameFact) -> dict[str, str]:
    return {
        "source": fact.source,
        "accession": fact.accession,
        "frame": fact.frame,
        "end": fact.end,
    }


def shares_provenance(fact: SharesFact) -> dict[str, Any]:
    provenance: dict[str, Any] = {
        "source": fact.source,
        "accession": fact.accession,
        "end": fact.end,
        "method": fact.method,
        "confidence": fact.confidence,
    }
    if fact.filed:
        provenance["filed"] = fact.filed
    if fact.form:
        provenance["form"] = fact.form
    if fact.frame:
        provenance["frame"] = fact.frame
    if fact.basis:
        provenance["basis"] = fact.basis
    if fact.policy_source:
        provenance["policy_source"] = fact.policy_source
    if fact.components:
        component_records: list[dict[str, Any]] = []
        for index, component in enumerate(fact.components):
            record: dict[str, Any] = {
                "tag": component.tag,
                "taxonomy": component.taxonomy,
                "value": component.value,
                "quarters": component.quarters,
                "segments": [
                    {"axis": axis, "member": member}
                    for axis, member in component.segments
                ],
            }
            if fact.component_multipliers:
                multiplier = fact.component_multipliers[index]
                record["multiplier"] = multiplier
                record["equivalent_shares"] = float(component.value) * multiplier
            component_records.append(record)
        provenance["components"] = component_records
    return provenance


def build_companies(
    identities: dict[int, dict[str, Any]],
    sic_facts: dict[int, SicFact],
    float_facts: dict[int, FrameFact],
    shares_facts: dict[int, SharesFact],
) -> list[dict[str, Any]]:
    by_sector: dict[str, list[dict[str, Any]]] = {sector: [] for sector in SECTORS}
    for cik, identity in identities.items():
        sic_fact = sic_facts.get(cik)
        float_fact = float_facts.get(cik)
        if sic_fact is None or float_fact is None:
            continue
        shares_fact = shares_facts.get(cik)
        if not public_float_passes_sanity(cik, float_fact, shares_fact, sic_fact):
            continue
        sector = sector_for_sic(sic_fact.sic)
        quality = "public_float_and_shares" if shares_fact else "public_float_only"
        provenance: dict[str, Any] = {
            "identity": {"source": "sec_company_tickers_exchange"},
            "sic": {
                "source": sic_fact.source,
                "accession": sic_fact.accession,
                "filed": sic_fact.filed,
                "form": sic_fact.form,
                "accelerated_filer_status": sic_fact.accelerated_filer_status,
            },
            "public_float": {
                **fact_provenance(float_fact),
                "confidence": "low",
                "sanity_screen": public_float_sanity_screen(cik, shares_fact),
            },
        }
        if shares_fact:
            provenance["shares_outstanding"] = shares_provenance(shares_fact)
        by_sector[sector].append(
            {
                "cik": f"{cik:010d}",
                "symbol": identity["symbol"],
                "name": identity["name"],
                "exchange": identity["exchange"],
                "sic": sic_fact.sic,
                "sector": sector,
                "public_float": float_fact.value,
                "proxy_source": float_fact.source,
                "proxy_as_of": float_fact.end,
                "proxy_confidence": "low",
                "proxy_sanity_screen": public_float_sanity_screen(
                    cik, shares_fact
                ),
                "shares_outstanding": shares_fact.value if shares_fact else None,
                "shares_source": shares_fact.source if shares_fact else None,
                "shares_as_of": shares_fact.end if shares_fact else None,
                "shares_confidence": (
                    shares_fact.confidence if shares_fact else None
                ),
                "shares_method": shares_fact.method if shares_fact else None,
                "as_of": float_fact.end,
                "quality": quality,
                "provenance": provenance,
            }
        )

    selected: list[dict[str, Any]] = []
    used_symbols: set[str] = set()
    for sector in SECTORS:
        candidates = sorted(
            by_sector[sector],
            key=lambda company: (-float(company["public_float"]), company["symbol"]),
        )
        sector_companies: list[dict[str, Any]] = []
        for company in candidates:
            if company["symbol"] in used_symbols:
                continue
            company["rank"] = len(sector_companies) + 1
            sector_companies.append(company)
            used_symbols.add(company["symbol"])
            if len(sector_companies) == TARGET_COMPANIES_PER_SECTOR:
                break
        if len(sector_companies) < MIN_COMPANIES_PER_SECTOR:
            raise RuntimeError(
                f"sector {sector} has only {len(sector_companies)} eligible unique issuers "
                f"({len(candidates)} before symbol deduplication)"
            )
        selected.extend(sector_companies)
    return selected


def public_float_passes_sanity(
    cik: int,
    public_float: FrameFact,
    shares_outstanding: SharesFact | None,
    sic_fact: SicFact,
) -> bool:
    value = float(public_float.value)
    if value > MAX_REPORTED_PUBLIC_FLOAT:
        return False
    status = sic_fact.accelerated_filer_status
    if status == "2-ACC" and value > MAX_ACCELERATED_FILER_FLOAT:
        return False
    if status in {"3-SRA", "4-NON"} and value > MAX_NON_ACCELERATED_FILER_FLOAT:
        return False
    if (
        not status
        and value > MAX_ACCELERATED_FILER_FLOAT
        and cik not in REVIEWED_LARGE_FLOAT_WITHOUT_AFS_CIKS
    ):
        return False
    if shares_outstanding is not None and cik not in REVIEWED_HIGH_PRICE_CIKS:
        implied_price = value / float(shares_outstanding.value)
        if implied_price > MAX_UNREVIEWED_IMPLIED_SHARE_PRICE:
            return False
    return True


def public_float_sanity_screen(
    cik: int, shares_outstanding: SharesFact | None
) -> str:
    if cik in REVIEWED_LARGE_FLOAT_WITHOUT_AFS_CIKS:
        return "reviewed_large_float_without_afs"
    if shares_outstanding is None:
        return "absolute_and_filer_status"
    if cik in REVIEWED_HIGH_PRICE_CIKS:
        return "reviewed_high_price_issuer"
    return "absolute_filer_status_and_implied_price"


def unresolved_share_reasons(
    companies: list[dict[str, Any]],
    reviewed_policies: dict[int, dict[str, Any]],
    latest_submissions: dict[int, dict[str, str | int]],
    cover_components: dict[int, list[ShareComponent]],
) -> dict[int, str]:
    reasons: dict[int, str] = {}
    for company in companies:
        if company["shares_outstanding"] is not None:
            continue
        cik = int(company["cik"])
        if reviewed_multiclass_issuer(cik, reviewed_policies):
            reason = "policy_signature_changed"
        elif cik not in latest_submissions:
            reason = "no_recent_inline_filing"
        elif not cover_components.get(cik):
            reason = "no_filing_cover_share_fact"
        elif len(latest_filing_cover_components(cover_components[cik])) > 1:
            reason = "multi_class_unreviewed"
        else:
            reason = "no_recent_unambiguous_price_equivalent_share_basis"
        reasons[cik] = reason
    return reasons


def share_coverage(
    companies: list[dict[str, Any]],
    unresolved_reasons: dict[int, str] | None = None,
) -> dict[str, Any]:
    unresolved_reasons = unresolved_reasons or {}
    unresolved = [
        company for company in companies if company["shares_outstanding"] is None
    ]
    top_companies = [company for company in companies if company["rank"] <= 100]
    unresolved_top = [
        company for company in top_companies if company["shares_outstanding"] is None
    ]
    return {
        "catalog_companies": len(companies),
        "catalog_resolved": len(companies) - len(unresolved),
        "catalog_unresolved": len(unresolved),
        "top_100_companies": len(top_companies),
        "top_100_resolved": len(top_companies) - len(unresolved_top),
        "top_100_unresolved": len(unresolved_top),
        "unresolved": [
            {
                "cik": company["cik"],
                "symbol": company["symbol"],
                "sector": company["sector"],
                "rank": company["rank"],
                "reason": unresolved_reasons.get(
                    int(company["cik"]),
                    "no_recent_unambiguous_price_equivalent_share_basis",
                ),
            }
            for company in unresolved
        ],
    }


def validate_catalog(
    companies: list[dict[str, Any]],
    unresolved_reasons: dict[int, str] | None = None,
) -> None:
    if len({company["cik"] for company in companies}) != len(companies):
        raise RuntimeError("catalog contains duplicate issuer CIKs")
    if len({company["symbol"] for company in companies}) != len(companies):
        raise RuntimeError("catalog contains duplicate canonical symbols")
    for sector in SECTORS:
        sector_rows = [company for company in companies if company["sector"] == sector]
        ranks = [company["rank"] for company in sector_rows]
        if not MIN_COMPANIES_PER_SECTOR <= len(ranks) <= TARGET_COMPANIES_PER_SECTOR:
            raise RuntimeError(f"sector {sector} has an invalid candidate count")
        if ranks != list(range(1, len(ranks) + 1)):
            raise RuntimeError(f"sector {sector} ranks are not consecutive")
    validate_top_100_share_coverage(companies, unresolved_reasons)


def validate_top_100_share_coverage(
    companies: list[dict[str, Any]],
    unresolved_reasons: dict[int, str] | None = None,
) -> None:
    unresolved_reasons = unresolved_reasons or {}
    unresolved_top = [
        (
            company["symbol"],
            unresolved_reasons.get(
                int(company["cik"]),
                "unresolved",
            ),
        )
        for company in companies
        if company["rank"] <= 100 and company["shares_outstanding"] is None
    ]
    if unresolved_top:
        raise RuntimeError(
            "top-100 share coverage regression: "
            + ", ".join(
                f"{symbol} ({reason})" for symbol, reason in unresolved_top
            )
        )


def runtime_catalog(catalog: dict[str, Any]) -> dict[str, Any]:
    """Project the audit catalog onto the fields consumed by the Rust client."""

    runtime = {
        field: required_field(catalog, field, "catalog")
        for field in (
            "schema_version",
            "catalog_version",
            "generated_at",
            "as_of",
        )
    }
    companies = required_field(catalog, "companies", "catalog")
    if not isinstance(companies, list):
        raise RuntimeError("catalog companies must be a list")
    runtime["companies"] = [
        runtime_company(company, index)
        for index, company in enumerate(companies)
    ]
    return runtime


def runtime_company(value: Any, index: int) -> dict[str, Any]:
    location = f"catalog company {index}"
    if not isinstance(value, dict):
        raise RuntimeError(f"{location} must be an object")
    company = {
        field: required_field(value, field, location)
        for field in (
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
        )
    }
    provenance = required_field(value, "provenance", location)
    if not isinstance(provenance, dict):
        raise RuntimeError(f"{location} provenance must be an object")
    public_float = runtime_fact_provenance(
        required_field(provenance, "public_float", f"{location} provenance"),
        f"{location} public-float provenance",
    )
    shares_value = company["shares_outstanding"]
    shares_provenance = provenance.get("shares_outstanding")
    if (shares_value is None) != (shares_provenance is None):
        raise RuntimeError(
            f"{location} shares value and provenance must both be present or absent"
        )
    company["provenance"] = {
        "public_float": public_float,
        "shares_outstanding": (
            None
            if shares_provenance is None
            else runtime_fact_provenance(
                shares_provenance,
                f"{location} shares provenance",
                optional_fields=("method", "confidence"),
            )
        ),
    }
    return company


def runtime_fact_provenance(
    value: Any,
    location: str,
    *,
    optional_fields: tuple[str, ...] = ("confidence",),
) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise RuntimeError(f"{location} must be an object")
    result = {
        field: required_field(value, field, location)
        for field in ("source", "end")
    }
    for field in optional_fields:
        if field in value and value[field] is not None:
            result[field] = value[field]
    return result


def required_field(value: dict[str, Any], field: str, location: str) -> Any:
    if field not in value:
        raise RuntimeError(f"{location} is missing {field}")
    return value[field]


def canonical_json_bytes(value: Any) -> bytes:
    try:
        serialized = json.dumps(
            value,
            ensure_ascii=True,
            allow_nan=False,
            sort_keys=True,
            separators=(",", ":"),
        )
    except (TypeError, ValueError) as error:
        raise RuntimeError("catalog contains a non-JSON value") from error
    return (serialized + "\n").encode("utf-8")


def deterministic_gzip(payload: bytes) -> bytes:
    """Compress bytes without embedding a filename or wall-clock timestamp."""

    output = io.BytesIO()
    with gzip.GzipFile(
        filename="",
        mode="wb",
        compresslevel=9,
        fileobj=output,
        mtime=0,
    ) as archive:
        archive.write(payload)
    return output.getvalue()


def artifact_manifest(
    catalog: dict[str, Any],
    artifact_path: Path,
    compressed: bytes,
    payload: bytes,
) -> dict[str, Any]:
    companies = catalog["companies"]
    sector_counts = {
        sector: sum(company["sector"] == sector for company in companies)
        for sector in SECTORS
    }
    return {
        "manifest_version": ARTIFACT_MANIFEST_VERSION,
        "catalog": {
            "schema_version": catalog["schema_version"],
            "catalog_version": catalog["catalog_version"],
            "generated_at": catalog["generated_at"],
            "as_of": catalog["as_of"],
            "company_count": len(companies),
            "sector_counts": sector_counts,
        },
        "artifact": {
            "filename": artifact_path.name,
            "compression": "gzip",
            "content_type": "application/json",
            "content_encoding": "gzip",
            "size_bytes": len(compressed),
            "sha256": hashlib.sha256(compressed).hexdigest(),
            "payload_size_bytes": len(payload),
            "payload_sha256": hashlib.sha256(payload).hexdigest(),
        },
    }


def default_manifest_path(artifact_path: Path) -> Path:
    suffix = ".json.gz"
    name = artifact_path.name
    stem = name[: -len(suffix)] if name.endswith(suffix) else artifact_path.stem
    return artifact_path.with_name(f"{stem}.manifest.json")


def write_runtime_catalog_artifact(
    catalog: dict[str, Any],
    artifact_path: Path,
    manifest_path: Path | None = None,
) -> tuple[dict[str, Any], Path]:
    runtime = runtime_catalog(catalog)
    payload = canonical_json_bytes(runtime)
    compressed = deterministic_gzip(payload)
    resolved_manifest_path = manifest_path or default_manifest_path(artifact_path)
    if artifact_path.resolve() == resolved_manifest_path.resolve():
        raise RuntimeError("artifact and manifest paths must be different")
    manifest = artifact_manifest(runtime, artifact_path, compressed, payload)

    atomic_write_bytes(artifact_path, compressed)
    atomic_write_bytes(
        resolved_manifest_path,
        json.dumps(
            manifest,
            indent=2,
            ensure_ascii=True,
            allow_nan=False,
            sort_keys=True,
        ).encode("utf-8")
        + b"\n",
    )
    return manifest, resolved_manifest_path


def atomic_write_bytes(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.tmp")
    temporary.write_bytes(payload)
    temporary.replace(path)


def load_catalog(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise RuntimeError(f"could not read catalog {path}: {error}") from error
    except json.JSONDecodeError as error:
        raise RuntimeError(f"catalog {path} is invalid JSON") from error
    if not isinstance(value, dict):
        raise RuntimeError(f"catalog {path} must contain a JSON object")
    return value


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--user-agent",
        default=os.environ.get("SEC_USER_AGENT"),
        help="SEC-compliant application/contact User-Agent (or SEC_USER_AGENT)",
    )
    parser.add_argument("--through", type=Quarter.parse, default=Quarter.current())
    parser.add_argument("--frame-quarters", type=int, default=12)
    parser.add_argument("--sic-quarters", type=int, default=2)
    parser.add_argument("--requests-per-second", type=float, default=8.0)
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=Path.home() / ".cache" / "stock-tui" / "sec-catalog",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "data" / "sec_universe.json",
    )
    parser.add_argument(
        "--artifact-output",
        type=Path,
        help=(
            "also write a compact deterministic .json.gz runtime catalog; source "
            "downloads and audit-only metadata are excluded"
        ),
    )
    parser.add_argument(
        "--artifact-manifest-output",
        type=Path,
        help=(
            "write artifact metadata and SHA-256 hashes here (defaults beside "
            "--artifact-output)"
        ),
    )
    parser.add_argument(
        "--package-only",
        action="store_true",
        help=(
            "package the existing --output catalog without contacting the SEC; "
            "requires --artifact-output"
        ),
    )
    arguments = parser.parse_args()
    if not arguments.package_only and not arguments.user_agent:
        parser.error(
            "--user-agent or SEC_USER_AGENT is required by SEC fair-access policy"
        )
    if arguments.package_only and arguments.artifact_output is None:
        parser.error("--package-only requires --artifact-output")
    if (
        arguments.artifact_manifest_output is not None
        and arguments.artifact_output is None
    ):
        parser.error("--artifact-manifest-output requires --artifact-output")
    if (
        arguments.artifact_output is not None
        and not arguments.artifact_output.name.endswith(".json.gz")
    ):
        parser.error("--artifact-output must end with .json.gz")
    if (
        arguments.artifact_output is not None
        and arguments.output.resolve() == arguments.artifact_output.resolve()
    ):
        parser.error("--output and --artifact-output must be different paths")
    if arguments.frame_quarters < 1 or arguments.sic_quarters < 1:
        parser.error("quarter counts must be positive")
    if not 0 < arguments.requests_per_second <= 10:
        parser.error("--requests-per-second must be greater than zero and at most 10")
    return arguments


def main() -> int:
    args = parse_args()
    if args.package_only:
        catalog = load_catalog(args.output)
        companies = catalog.get("companies")
        if not isinstance(companies, list):
            raise RuntimeError("catalog companies must be a list")
        validate_top_100_share_coverage(companies)
        manifest, manifest_path = write_runtime_catalog_artifact(
            catalog,
            args.artifact_output,
            args.artifact_manifest_output,
        )
        print(
            f"packaged {manifest['catalog']['company_count']} companies to "
            f"{args.artifact_output} with manifest {manifest_path}",
            file=sys.stderr,
        )
        return 0

    client = SecClient(args.user_agent, args.requests_per_second, args.cache_dir)
    generated_at = utc_now()
    generated_on = date.fromisoformat(generated_at[:10])
    reviewed_policies = load_reviewed_share_policies()

    latest = find_latest_fsds(client, args.through)
    identities, identity_source = load_tickers(client)
    sic_facts, sic_sources = load_sic_facts(
        client, quarter_sequence(latest, args.sic_quarters)
    )
    # Frames are published independently of the quarterly FSDS archive. Search
    # from the requested boundary so a not-yet-published FSDS quarter does not
    # hide already available current-quarter frame facts.
    frame_quarters = quarter_sequence(args.through, args.frame_quarters)
    float_facts, float_sources = load_frame_facts(
        client, frame_quarters, PUBLIC_FLOAT_TAG, "USD", generated_on
    )
    frame_shares_facts, frame_shares_sources = load_frame_facts(
        client, frame_quarters, SHARES_TAG, "shares", generated_on
    )
    common_frame_shares_facts, common_frame_shares_sources = load_frame_facts(
        client,
        frame_quarters,
        COMMON_SHARES_TAG,
        "shares",
        generated_on,
        taxonomy="us-gaap",
    )
    (
        fsds_shares_facts,
        fsds_shares_sources,
        fsds_share_components,
    ) = load_fsds_share_facts(
        client,
        quarter_sequence(latest, args.sic_quarters),
        identities,
        reviewed_policies,
    )
    initial_shares_facts = merge_share_facts(
        frame_shares_facts,
        fsds_shares_facts,
        common_frame_shares_facts,
        reviewed_policies,
        generated_on,
    )
    initial_companies = build_companies(
        identities,
        sic_facts,
        float_facts,
        initial_shares_facts,
    )
    cover_targets = {
        int(company["cik"])
        for company in initial_companies
        if company["shares_outstanding"] is None
        or company["shares_method"] == "fsds_limited_partners_weighted_average"
    }
    cover_targets.update(
        cik
        for cik in identities
        if reviewed_multiclass_issuer(cik, reviewed_policies)
    )
    latest_submissions, submission_sources = load_latest_inline_submissions(
        client,
        cover_targets,
        generated_on,
    )
    cover_share_components, cover_sources = load_filing_cover_share_components(
        client,
        latest_submissions,
        generated_on,
        reviewed_policies,
    )
    for cik, components in cover_share_components.items():
        fsds_share_components.setdefault(cik, []).extend(components)

    filing_shares_facts: dict[int, SharesFact] = {}
    for cik, components in fsds_share_components.items():
        identity = identities.get(cik)
        if identity is None:
            continue
        if (
            reviewed_multiclass_issuer(cik, reviewed_policies)
            and cik not in cover_share_components
        ):
            continue
        selected = select_fsds_shares_fact(
            cik,
            str(identity["symbol"]),
            components,
            reviewed_policies,
        )
        if selected is not None:
            filing_shares_facts[cik] = selected
    shares_facts = merge_share_facts(
        frame_shares_facts,
        filing_shares_facts,
        common_frame_shares_facts,
        reviewed_policies,
        generated_on,
    )
    companies = build_companies(identities, sic_facts, float_facts, shares_facts)
    unresolved_reasons = unresolved_share_reasons(
        companies,
        reviewed_policies,
        latest_submissions,
        cover_share_components,
    )
    validate_catalog(companies, unresolved_reasons)

    sources = [source_record(identity_source, TICKERS_URL, client)]
    sources.extend(sic_sources)
    sources.extend(float_sources)
    sources.extend(frame_shares_sources)
    sources.extend(common_frame_shares_sources)
    sources.extend(fsds_shares_sources)
    sources.extend(submission_sources)
    sources.extend(cover_sources)
    coverage = share_coverage(companies, unresolved_reasons)
    catalog = {
        "schema_version": SCHEMA_VERSION,
        "catalog_version": (
            f"sec-universe-v{SCHEMA_VERSION}-fsds-{latest.label().lower()}-"
            f"frames-{args.through.label().lower()}"
        ),
        "generated_at": generated_at,
        "as_of": max(company["as_of"] for company in companies),
        "selection": {
            "latest_fsds": latest.label(),
            "frame_search_through": args.through.label(),
            "minimum_companies_per_sector": MIN_COMPANIES_PER_SECTOR,
            "target_companies_per_sector": TARGET_COMPANIES_PER_SECTOR,
            "issuer_identity": (
                "unique SEC CIK with one deterministic canonical exchange ticker; "
                "prefer common-shaped symbols, then a concise source-prefix base, "
                "then SEC order"
            ),
            "ranking_proxy": "SEC dei:EntityPublicFloat (USD), descending within sector",
            "market_cap_warning": (
                "EntityPublicFloat is issuer-level reported public float, not market "
                "capitalization. Compute market cap only from shares outstanding and a "
                "contemporaneous market price."
            ),
            "sector_mapping": "SEC SIC mapped to StockTouch's nine legacy sectors",
            "quality_values": {
                "public_float_and_shares": (
                    "public float and an SEC shares estimate were available"
                ),
                "public_float_only": "ranking fact available; shares fact unavailable",
            },
            "shares_fallback": (
                "Recent filing-level SEC cover facts with exact reviewed class "
                "signatures; SEC DEI issuer total; reviewed DEI class sum; US-GAAP "
                "common-stock issuer total; reviewed equal-economic class sum; "
                "filer-reported equivalent class; basic weighted-average common "
                "shares or partnership units. Preferred and diluted securities are "
                "excluded."
            ),
            "reviewed_share_policy_registry": str(
                SHARE_POLICY_PATH.relative_to(Path(__file__).resolve().parents[1])
            ),
            "share_coverage": coverage,
            "quality_screening": (
                "Excludes non-positive facts, extreme absolute values, implausible "
                "public-float/filer-status combinations, and isolated greater-than-"
                "100x upward year-over-year jumps. Downward corrections are retained, "
                "and a $2,000 implied-price gross-error guard has reviewed high-price "
                "issuer exceptions. Screening does not alter reported SEC values."
            ),
        },
        "sources": sources,
        "companies": companies,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = args.output.with_suffix(args.output.suffix + ".tmp")
    temporary.write_text(
        json.dumps(catalog, indent=2, ensure_ascii=True) + "\n", encoding="utf-8"
    )
    temporary.replace(args.output)
    print(
        f"wrote {len(companies)} companies from {len(sources)} SEC sources to {args.output}",
        file=sys.stderr,
    )
    if args.artifact_output is not None:
        manifest, manifest_path = write_runtime_catalog_artifact(
            catalog,
            args.artifact_output,
            args.artifact_manifest_output,
        )
        print(
            f"packaged {manifest['catalog']['company_count']} companies to "
            f"{args.artifact_output} with manifest {manifest_path}",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
