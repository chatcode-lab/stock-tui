#!/usr/bin/env python3
"""Build stock-tui's embedded issuer universe from official SEC data.

The output deliberately stores SEC EntityPublicFloat as a ranking proxy, not as
market capitalization. A runtime market cap requires a contemporaneous price
multiplied by shares outstanding.
"""

from __future__ import annotations

import argparse
import csv
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
from dataclasses import dataclass
from datetime import date, datetime, timezone
from pathlib import Path
from typing import Any, Iterable


TICKERS_URL = "https://www.sec.gov/files/company_tickers_exchange.json"
FSDS_URL = (
    "https://www.sec.gov/files/dera/data/financial-statement-data-sets/"
    "{year}q{quarter}.zip"
)
FRAME_URL = (
    "https://data.sec.gov/api/xbrl/frames/dei/{tag}/{unit}/CY{year}Q{quarter}I.json"
)
PUBLIC_FLOAT_TAG = "EntityPublicFloat"
SHARES_TAG = "EntityCommonStockSharesOutstanding"
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
SCHEMA_VERSION = 1
MIN_COMPANIES_PER_SECTOR = 100
TARGET_COMPANIES_PER_SECTOR = 250
MAX_REPORTED_PUBLIC_FLOAT = 5_000_000_000_000
MAX_IMPLIED_SHARE_PRICE = 1_000


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
    source: str


@dataclass(frozen=True)
class FrameFact:
    value: int | float
    end: str
    accession: str
    frame: str
    source: str


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
            or not symbol.isascii()
        ):
            continue
        grouped.setdefault(cik, []).append(
            {"symbol": symbol, "name": name, "exchange": exchange, "ordinal": ordinal}
        )

    identities = {
        cik: min(values, key=canonical_symbol_key) for cik, values in grouped.items()
    }
    return identities, "sec_company_tickers_exchange"


def canonical_symbol_key(identity: dict[str, Any]) -> tuple[int, int]:
    """Prefer likely common stock, then preserve the SEC file's issuer order."""
    symbol = identity["symbol"]
    suffix = (
        symbol.replace(".", "-").rsplit("-", 1)[-1]
        if "-" in symbol or "." in symbol
        else ""
    )
    derivative_penalty = int(bool(suffix) and suffix.startswith(("P", "W", "U", "R")))
    return derivative_penalty, int(identity["ordinal"])


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
                    candidate = SicFact(sic, accession, filed, form, source_id)
                    existing = facts.get(cik)
                    if existing is None or (candidate.filed, candidate.accession) > (
                        existing.filed,
                        existing.accession,
                    ):
                        facts[cik] = candidate
    return facts, sources


def load_frame_facts(
    client: SecClient,
    quarters: Iterable[Quarter],
    tag: str,
    unit: str,
) -> tuple[dict[int, FrameFact], list[dict[str, str]]]:
    candidates: dict[int, list[FrameFact]] = {}
    sources: list[dict[str, str]] = []
    for quarter in quarters:
        frame = f"CY{quarter.year}Q{quarter.quarter}I"
        url = FRAME_URL.format(
            tag=tag, unit=unit, year=quarter.year, quarter=quarter.quarter
        )
        payload = client.get(url, optional=True)
        if payload is None:
            continue
        source_id = f"sec_frame_{snake_case(tag)}_{frame}"
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
            end = str(row.get("end") or "")
            if value is None or not end:
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
        if ratio > 100 or ratio < 0.01:
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


def build_companies(
    identities: dict[int, dict[str, Any]],
    sic_facts: dict[int, SicFact],
    float_facts: dict[int, FrameFact],
    shares_facts: dict[int, FrameFact],
) -> list[dict[str, Any]]:
    by_sector: dict[str, list[dict[str, Any]]] = {sector: [] for sector in SECTORS}
    for cik, identity in identities.items():
        sic_fact = sic_facts.get(cik)
        float_fact = float_facts.get(cik)
        if sic_fact is None or float_fact is None:
            continue
        shares_fact = shares_facts.get(cik)
        if not public_float_passes_sanity(float_fact, shares_fact):
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
            },
            "public_float": fact_provenance(float_fact),
        }
        if shares_fact:
            provenance["shares_outstanding"] = fact_provenance(shares_fact)
        by_sector[sector].append(
            {
                "cik": f"{cik:010d}",
                "symbol": identity["symbol"],
                "name": identity["name"],
                "exchange": identity["exchange"],
                "sic": sic_fact.sic,
                "sector": sector,
                "public_float": float_fact.value,
                "shares_outstanding": shares_fact.value if shares_fact else None,
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
    public_float: FrameFact, shares_outstanding: FrameFact | None
) -> bool:
    value = float(public_float.value)
    if value > MAX_REPORTED_PUBLIC_FLOAT:
        return False
    if shares_outstanding is None:
        return True
    shares = float(shares_outstanding.value)
    implied_price = value / shares
    return implied_price <= MAX_IMPLIED_SHARE_PRICE


def validate_catalog(companies: list[dict[str, Any]]) -> None:
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
    arguments = parser.parse_args()
    if not arguments.user_agent:
        parser.error(
            "--user-agent or SEC_USER_AGENT is required by SEC fair-access policy"
        )
    if arguments.frame_quarters < 1 or arguments.sic_quarters < 1:
        parser.error("quarter counts must be positive")
    if not 0 < arguments.requests_per_second <= 10:
        parser.error("--requests-per-second must be greater than zero and at most 10")
    return arguments


def main() -> int:
    args = parse_args()
    client = SecClient(args.user_agent, args.requests_per_second, args.cache_dir)
    generated_at = utc_now()

    latest = find_latest_fsds(client, args.through)
    identities, identity_source = load_tickers(client)
    sic_facts, sic_sources = load_sic_facts(
        client, quarter_sequence(latest, args.sic_quarters)
    )
    frame_quarters = quarter_sequence(latest, args.frame_quarters)
    float_facts, float_sources = load_frame_facts(
        client, frame_quarters, PUBLIC_FLOAT_TAG, "USD"
    )
    shares_facts, shares_sources = load_frame_facts(
        client, frame_quarters, SHARES_TAG, "shares"
    )
    companies = build_companies(identities, sic_facts, float_facts, shares_facts)
    validate_catalog(companies)

    sources = [source_record(identity_source, TICKERS_URL, client)]
    sources.extend(sic_sources)
    sources.extend(float_sources)
    sources.extend(shares_sources)
    catalog = {
        "schema_version": SCHEMA_VERSION,
        "catalog_version": f"sec-universe-v{SCHEMA_VERSION}-{latest.label().lower()}",
        "generated_at": generated_at,
        "as_of": max(company["as_of"] for company in companies),
        "selection": {
            "minimum_companies_per_sector": MIN_COMPANIES_PER_SECTOR,
            "target_companies_per_sector": TARGET_COMPANIES_PER_SECTOR,
            "issuer_identity": "unique SEC CIK with one deterministic canonical exchange ticker",
            "ranking_proxy": "SEC dei:EntityPublicFloat (USD), descending within sector",
            "market_cap_warning": (
                "EntityPublicFloat is issuer-level reported public float, not market "
                "capitalization. Compute market cap only from shares outstanding and a "
                "contemporaneous market price."
            ),
            "sector_mapping": "SEC SIC mapped to StockTouch's nine legacy sectors",
            "quality_values": {
                "public_float_and_shares": "both requested SEC facts were available",
                "public_float_only": "ranking fact available; shares fact unavailable",
            },
            "quality_screening": (
                "Excludes non-positive facts, extreme absolute values, implausible "
                "public-float-to-shares ratios, and isolated greater-than-100x "
                "year-over-year jumps. Screening does not alter reported SEC values."
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
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
