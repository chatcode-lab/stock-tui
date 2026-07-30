from __future__ import annotations

import json
import tempfile
import unittest
import urllib.error
from dataclasses import replace
from datetime import date
from pathlib import Path
from unittest import mock

from tools import build_sec_catalog as catalog


FIXTURE = Path(__file__).parent / "fixtures" / "sec_shares_cases.json"


def fixture_components(case: dict[str, object]) -> list[catalog.ShareComponent]:
    submission = {
        "accession": case["accession"],
        "filed": case["filed"],
        "form": case["form"],
    }
    components: list[catalog.ShareComponent] = []
    for tag, taxonomy, end, quarters, segments, value in case["rows"]:
        row = {
            "tag": tag,
            "version": taxonomy,
            "ddate": end,
            "qtrs": quarters,
            "uom": "shares",
            "segments": segments,
            "coreg": "",
            "value": value,
        }
        component = catalog.parse_fsds_share_component(
            row, submission, "fixture_fsds_num"
        )
        if component is not None:
            components.append(component)
    return components


def ticker_identity(
    symbol: str,
    ordinal: int,
    exchange: str = "Nasdaq",
) -> dict[str, object]:
    return {
        "symbol": symbol,
        "name": "Fixture issuer",
        "exchange": exchange,
        "ordinal": ordinal,
    }


def wikidata_binding(
    cik: str,
    item_id: str,
    label: str,
    *,
    description: str | None = None,
    industry: str | None = None,
) -> dict[str, dict[str, str]]:
    binding = {
        "cik": {"type": "literal", "value": cik},
        "item": {
            "type": "uri",
            "value": f"http://www.wikidata.org/entity/{item_id}",
        },
        "itemLabel": {
            "type": "literal",
            "xml:lang": "en",
            "value": label,
        },
    }
    if description is not None:
        binding["itemDescription"] = {
            "type": "literal",
            "xml:lang": "en",
            "value": description,
        }
    if industry is not None:
        binding["industryLabel"] = {
            "type": "literal",
            "xml:lang": "en",
            "value": industry,
        }
    return binding


def wikidata_payload(
    *bindings: dict[str, dict[str, str]],
) -> bytes:
    return json.dumps({"results": {"bindings": list(bindings)}}).encode()


def wikidata_search_binding(
    ordinal: int,
    item_id: str,
    label: str,
    *,
    business_type: str = catalog.WIKIDATA_BUSINESS_ITEM_ID,
    exchange_item: str = "Q82059",
    ticker: str = "EXM",
    listing_rank: str = "http://wikiba.se/ontology#NormalRank",
    listing_end: str | None = None,
    ended: str | None = None,
    parent: str | None = None,
    description: str | None = None,
    industry: str | None = None,
    product: str | None = None,
) -> dict[str, dict[str, str]]:
    binding = {
        "ordinal": {
            "type": "literal",
            "value": str(ordinal),
        },
        "item": {
            "type": "uri",
            "value": f"http://www.wikidata.org/entity/{item_id}",
        },
        "itemLabel": {
            "type": "literal",
            "xml:lang": "en",
            "value": label,
        },
        "businessType": {
            "type": "uri",
            "value": f"http://www.wikidata.org/entity/{business_type}",
        },
        "listingExchange": {
            "type": "uri",
            "value": f"http://www.wikidata.org/entity/{exchange_item}",
        },
        "listingTicker": {
            "type": "literal",
            "value": ticker,
        },
        "listingRank": {
            "type": "uri",
            "value": listing_rank,
        },
    }
    if listing_end is not None:
        binding["listingEnd"] = {
            "type": "literal",
            "value": listing_end,
        }
    if ended is not None:
        binding["ended"] = {
            "type": "literal",
            "value": ended,
        }
    if parent is not None:
        binding["parent"] = {
            "type": "uri",
            "value": f"http://www.wikidata.org/entity/{parent}",
        }
    if description is not None:
        binding["itemDescription"] = {
            "type": "literal",
            "xml:lang": "en",
            "value": description,
        }
    if industry is not None:
        binding["industryLabel"] = {
            "type": "literal",
            "xml:lang": "en",
            "value": industry,
        }
    if product is not None:
        binding["productLabel"] = {
            "type": "literal",
            "xml:lang": "en",
            "value": product,
        }
    return binding


class SicDescriptionTests(unittest.TestCase):
    def test_parses_documentation_labels_through_xbrl_arcs(self) -> None:
        payload = b"""<?xml version="1.0"?>
<xs:schema
  xmlns:xs="http://www.w3.org/2001/XMLSchema"
  xmlns:link="http://www.xbrl.org/2003/linkbase"
  xmlns:xlink="http://www.w3.org/1999/xlink">
  <link:label xlink:type="resource" xlink:label="lab_Z3571"
    xlink:role="http://www.xbrl.org/2003/role/documentation">
    Electronic   Computers
  </link:label>
  <link:labelArc xlink:type="arc"
    xlink:arcrole="http://www.xbrl.org/2003/arcrole/concept-label"
    xlink:from="loc_Z3571" xlink:to="lab_Z3571"/>
  <link:loc xlink:type="locator"
    xlink:href="sic-2026.xsd#sic_Z3571" xlink:label="loc_Z3571"/>
  <link:label xlink:type="resource" xlink:label="lab_standard"
    xlink:role="http://www.xbrl.org/2003/role/label">Ignored</link:label>
</xs:schema>"""

        self.assertEqual(
            catalog.parse_sic_descriptions(payload, "fixture"),
            {3571: "Electronic Computers"},
        )

    def test_rejects_conflicting_descriptions_for_one_sic(self) -> None:
        payload = b"""<?xml version="1.0"?>
<xs:schema
  xmlns:xs="http://www.w3.org/2001/XMLSchema"
  xmlns:link="http://www.xbrl.org/2003/linkbase"
  xmlns:xlink="http://www.w3.org/1999/xlink">
  <link:loc xlink:href="#sic_Z3571" xlink:label="loc_Z3571"/>
  <link:label xlink:label="lab_one"
    xlink:role="http://www.xbrl.org/2003/role/documentation">Computers</link:label>
  <link:label xlink:label="lab_two"
    xlink:role="http://www.xbrl.org/2003/role/documentation">Other Computers</link:label>
  <link:labelArc
    xlink:arcrole="http://www.xbrl.org/2003/arcrole/concept-label"
    xlink:from="loc_Z3571" xlink:to="lab_one"/>
  <link:labelArc
    xlink:arcrole="http://www.xbrl.org/2003/arcrole/concept-label"
    xlink:from="loc_Z3571" xlink:to="lab_two"/>
</xs:schema>"""

        with self.assertRaisesRegex(RuntimeError, "conflicting descriptions"):
            catalog.parse_sic_descriptions(payload, "fixture")

    def test_rejects_taxonomy_without_linked_descriptions(self) -> None:
        payload = b"""<?xml version="1.0"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema"/>"""

        with self.assertRaisesRegex(RuntimeError, "contains no descriptions"):
            catalog.parse_sic_descriptions(payload, "fixture")


class WikidataCompanyProfileTests(unittest.TestCase):
    def test_query_is_deterministic_and_uses_padded_and_unpadded_ciks(
        self,
    ) -> None:
        first = catalog.wikidata_company_query(["0001652044", "0000320193"])
        second = catalog.wikidata_company_query(["0000320193", "1652044"])

        self.assertEqual(first, second)
        self.assertIn("wdt:P5531", first)
        self.assertIn('"320193"', first)
        self.assertIn('"0000320193"', first)
        self.assertIn('"1652044"', first)
        self.assertIn('"0001652044"', first)

    def test_entity_search_query_is_bounded_structured_and_escaped(self) -> None:
        query = catalog.wikidata_entity_search_query(
            "AMAZON COM INC",
            "AMZN",
            "Nasdaq",
        )

        self.assertIn('wikibase:api "EntitySearch"', query)
        self.assertIn('mwapi:search "amazon"', query)
        self.assertEqual(
            catalog.wikidata_entity_search_term("AMAZON COM INC"),
            "amazon",
        )
        self.assertEqual(
            catalog.wikidata_entity_search_term(
                "BANK OF AMERICA CORP /DE/"
            ),
            "bank of america",
        )
        self.assertIn("wdt:P452", query)
        self.assertIn("wdt:P1056", query)
        self.assertIn(
            f"wdt:P31/wdt:P279* wd:{catalog.WIKIDATA_BUSINESS_ITEM_ID}",
            query,
        )
        self.assertIn("pq:P249 ?listingTicker", query)
        self.assertIn("wikibase:rank ?listingRank", query)
        self.assertIn(
            "FILTER(?listingRank != wikibase:DeprecatedRank)",
            query,
        )
        self.assertIn("VALUES ?listingExchange { wd:Q82059 }", query)
        self.assertIn('"AMZN"', query)
        self.assertIn(
            "COALESCE(?englishLabel, ?multilingualLabel)",
            query,
        )
        self.assertIn(
            f'mwapi:limit "{catalog.WIKIDATA_ENTITY_SEARCH_LIMIT}"',
            query,
        )
        self.assertTrue(
            query.rstrip().endswith(
                f"LIMIT {catalog.MAX_WIKIDATA_ENTITY_SEARCH_ROWS + 1}"
            )
        )

    def test_builds_readable_description_with_sorted_distinct_industries(
        self,
    ) -> None:
        payload = wikidata_payload(
            wikidata_binding(
                "0001045810",
                "Q182477",
                "Nvidia",
                description="American technology company",
                industry="technology industry",
            ),
            wikidata_binding(
                "1045810",
                "Q182477",
                "Nvidia",
                description="American technology company",
                industry="semiconductor industry",
            ),
            wikidata_binding(
                "1045810",
                "Q182477",
                "Nvidia",
                description="American technology company",
                industry="technology industry",
            ),
        )

        profiles = catalog.parse_wikidata_company_profiles(
            payload, {"0001045810": "NVIDIA CORP"}
        )

        self.assertEqual(
            profiles["0001045810"].description,
            "American technology company. Focus: semiconductors.",
        )
        self.assertEqual(
            profiles["0001045810"].source_url,
            "https://www.wikidata.org/wiki/Q182477",
        )
        self.assertEqual(
            profiles["0001045810"].industries,
            ("semiconductors",),
        )

    def test_industry_refinement_removes_taxonomy_noise_and_overlap(
        self,
    ) -> None:
        industries = (
            "International Standard Industrial Classification",
            "electronics industry",
            "consumer electronics industry",
            "cloud computing sector",
            "software industry",
            "software development",
            "technology industry",
        )

        self.assertEqual(
            catalog.refined_industry_labels(
                "American technology company",
                industries,
            ),
            (
                "cloud computing",
                "consumer electronics",
                "software development",
            ),
        )
        self.assertEqual(
            catalog.synthesize_company_description(
                "American technology company",
                industries,
            ),
            (
                "American technology company. Focus: cloud computing, "
                "consumer electronics, and software development."
            ),
        )

    def test_industry_refinement_normalizes_economics_of_banking(self) -> None:
        self.assertEqual(
            catalog.refined_industry_labels(
                "American investment bank",
                ("economics of banking", "financial services industry"),
            ),
            ("financial services",),
        )

    def test_generic_description_requires_meaningful_industry_context(self) -> None:
        self.assertIsNone(
            catalog.synthesize_company_description("company", ())
        )
        self.assertIsNone(
            catalog.synthesize_company_description(
                "American company",
                ("dental equipment industry",),
            )
        )
        self.assertEqual(
            catalog.synthesize_company_description(
                "American company",
                ("dental equipment industry", "medical technology industry"),
            ),
            "Business areas: dental equipment and medical technology.",
        )

    def test_promotional_description_is_replaced_by_factual_focus(self) -> None:
        self.assertIsNone(
            catalog.synthesize_company_description(
                (
                    "Innovative biopharmaceutical company focused on "
                    "transformative medicines"
                ),
                ("pharmaceutical industry",),
            )
        )

    def test_legal_and_jurisdiction_boilerplate_is_removed_or_rejected(
        self,
    ) -> None:
        self.assertEqual(
            catalog.cleaned_wikidata_description(
                "American insurance company incorporated in Zurich."
            ),
            "American insurance company.",
        )
        for description in (
            "Company incorporated in Delaware",
            "Corporation registered under Delaware law",
            "American holding company",
            "Public company based in New York",
            "Privately held American software company",
            "American company in Delaware",
            "Private corporation in Toronto",
            "Holding company in New York",
            "; incorporated in Delaware",
        ):
            with self.subTest(description=description):
                self.assertEqual(
                    catalog.cleaned_wikidata_description(description),
                    "",
                )
        self.assertEqual(
            catalog.cleaned_wikidata_description(
                "American software company in Seattle"
            ),
            "American software company in Seattle.",
        )

    def test_duplicate_cik_selects_only_one_exact_normalized_label(
        self,
    ) -> None:
        profiles = catalog.parse_wikidata_company_profiles(
            wikidata_payload(
                wikidata_binding(
                    "123",
                    "Q1",
                    "Example Holdings",
                    description="unrelated holding company",
                ),
                wikidata_binding(
                    "0000000123",
                    "Q2",
                    "Example, Inc.",
                    description="American software company",
                ),
            ),
            {"0000000123": "EXAMPLE INC"},
        )

        self.assertEqual(profiles["0000000123"].item_id, "Q2")

    def test_duplicate_cik_name_match_ignores_common_legal_suffix(self) -> None:
        profiles = catalog.parse_wikidata_company_profiles(
            wikidata_payload(
                wikidata_binding(
                    "1571996",
                    "Q30873",
                    "Dell",
                    description="unrelated item",
                ),
                wikidata_binding(
                    "0001571996",
                    "Q30872",
                    "Dell Technologies",
                    description="American technology company",
                ),
            ),
            {"0001571996": "DELL TECHNOLOGIES INC."},
        )

        self.assertEqual(profiles["0001571996"].item_id, "Q30872")

    def test_duplicate_cik_without_unique_exact_label_is_skipped(self) -> None:
        profiles = catalog.parse_wikidata_company_profiles(
            wikidata_payload(
                wikidata_binding(
                    "123",
                    "Q1",
                    "Example Holdings",
                    description="holding company",
                ),
                wikidata_binding(
                    "123",
                    "Q2",
                    "Example Software",
                    description="software company",
                ),
            ),
            {"0000000123": "EXAMPLE INC"},
        )

        self.assertEqual(profiles, {})

    def test_missing_profile_coverage_is_valid(self) -> None:
        self.assertEqual(
            catalog.parse_wikidata_company_profiles(
                wikidata_payload(),
                {"0000000123": "Example Inc."},
            ),
            {},
        )

    def test_amazon_legal_stub_falls_back_to_canonical_company_profile(
        self,
    ) -> None:
        exact_payload = wikidata_payload(
            wikidata_binding(
                "0001018724",
                "Q133848906",
                "Amazon.com, Inc.",
                description="company incorporated in Delaware",
            )
        )
        search_payload = wikidata_payload(
            wikidata_search_binding(
                0,
                "Q3884",
                "Amazon",
                ticker="AMZN",
                description="American multinational technology company",
                industry="e-commerce",
                product="Amazon Web Services",
            ),
            wikidata_search_binding(
                1,
                "Q133848906",
                "Amazon.com, Inc.",
                ticker="AMZN",
                description="company incorporated in Delaware",
            ),
        )

        self.assertEqual(
            catalog.parse_wikidata_company_profiles(
                exact_payload,
                {"0001018724": "AMAZON COM INC"},
            ),
            {},
        )
        profile = catalog.parse_wikidata_entity_search_profile(
            search_payload,
            "AMAZON COM INC",
            "AMZN",
            "Nasdaq",
        )

        self.assertIsNotNone(profile)
        assert profile is not None
        self.assertEqual(profile.item_id, "Q3884")
        self.assertEqual(
            profile.description,
            (
                "American multinational technology company. Focus: e-commerce. "
                "Products and services: Amazon Web Services."
            ),
        )
        self.assertEqual(profile.industries, ("e-commerce",))
        self.assertEqual(profile.products, ("Amazon Web Services",))

    def test_entity_search_accepts_two_structured_facts_without_a_stub(
        self,
    ) -> None:
        profile = catalog.parse_wikidata_entity_search_profile(
            wikidata_payload(
                wikidata_search_binding(
                    0,
                    "Q10",
                    "Example",
                    product="industrial robots",
                ),
                wikidata_search_binding(
                    0,
                    "Q10",
                    "Example",
                    product="factory software",
                ),
            ),
            "Example Inc.",
            "EXM",
            "Nasdaq",
        )

        self.assertIsNotNone(profile)
        assert profile is not None
        self.assertEqual(
            profile.description,
            "Products and services: factory software and industrial robots.",
        )

    def test_entity_search_accepts_unique_top_corporate_name_shortening(
        self,
    ) -> None:
        profile = catalog.parse_wikidata_entity_search_profile(
            wikidata_payload(
                wikidata_search_binding(
                    0,
                    "Q173395",
                    "Cisco",
                    ticker="CSCO",
                    description="American digital communications company",
                    industry="networking hardware",
                ),
                wikidata_search_binding(
                    1,
                    "Q20",
                    "Cisco Systems India",
                    ticker="CSCO",
                    description="Indian subsidiary",
                ),
            ),
            "CISCO SYSTEMS INC",
            "CSCO",
            "Nasdaq",
        )

        self.assertIsNotNone(profile)
        assert profile is not None
        self.assertEqual(profile.item_id, "Q173395")
        self.assertEqual(
            profile.description,
            (
                "American digital communications company. "
                "Focus: networking hardware."
            ),
        )

    def test_entity_search_rejects_known_non_business_name_collisions(
        self,
    ) -> None:
        collisions = (
            ("BOX INC", "BOX", "Q895512", "Box", "Q43229", "English village"),
            (
                "BLOCK INC",
                "XYZ",
                "Q884653",
                "Block",
                "Q101352",
                "family name",
            ),
            (
                "MOSAIC CO",
                "MOS",
                "Q381047",
                "Mosaic",
                "Q131093",
                "early web browser",
            ),
            (
                "GAP INC",
                "GAP",
                "Q175081",
                "Gap",
                "Q484170",
                "French commune",
            ),
        )
        for issuer, symbol, item_id, label, item_type, description in collisions:
            with self.subTest(symbol=symbol):
                self.assertIsNone(
                    catalog.parse_wikidata_entity_search_profile(
                        wikidata_payload(
                            wikidata_search_binding(
                                0,
                                item_id,
                                label,
                                business_type=item_type,
                                exchange_item="Q13677",
                                ticker=symbol,
                                description=description,
                            )
                        ),
                        issuer,
                        symbol,
                        "NYSE",
                    )
                )

    def test_entity_search_accepts_unique_later_business_listing_match(
        self,
    ) -> None:
        profile = catalog.parse_wikidata_entity_search_profile(
            wikidata_payload(
                wikidata_search_binding(
                    0,
                    "Q175081",
                    "Gap",
                    business_type="Q484170",
                    exchange_item="Q13677",
                    ticker="GAP",
                    description="French commune",
                ),
                wikidata_search_binding(
                    3,
                    "Q420822",
                    "Gap Inc.",
                    exchange_item="Q13677",
                    ticker="GAP",
                    description="American clothing and accessories retailer",
                    industry="retail",
                ),
            ),
            "GAP INC",
            "GAP",
            "NYSE",
        )

        self.assertIsNotNone(profile)
        assert profile is not None
        self.assertEqual(profile.item_id, "Q420822")

    def test_entity_search_requires_active_compatible_independent_listing(
        self,
    ) -> None:
        incompatible_evidence = (
            {"ticker": "OTHER"},
            {"exchange_item": "Q13677"},
            {"listing_rank": "http://wikiba.se/ontology#DeprecatedRank"},
            {"listing_end": "2025-01-01T00:00:00Z"},
            {"ended": "2025-01-01T00:00:00Z"},
            {"parent": "Q2"},
        )
        for evidence in incompatible_evidence:
            with self.subTest(evidence=evidence):
                self.assertIsNone(
                    catalog.parse_wikidata_entity_search_profile(
                        wikidata_payload(
                            wikidata_search_binding(
                                0,
                                "Q1",
                                "Example",
                                description="American software company",
                                **evidence,
                            )
                        ),
                        "Example Inc.",
                        "EXM",
                        "Nasdaq",
                    )
                )

    def test_entity_search_accepts_dot_hyphen_ticker_alias(self) -> None:
        profile = catalog.parse_wikidata_entity_search_profile(
            wikidata_payload(
                wikidata_search_binding(
                    0,
                    "Q1",
                    "Example",
                    exchange_item="Q13677",
                    ticker="EXM-A",
                    description="American software company",
                )
            ),
            "Example Inc.",
            "EXM.A",
            "NYSE",
        )

        self.assertIsNotNone(profile)

    def test_entity_search_rejects_ambiguous_or_unrelated_candidates(
        self,
    ) -> None:
        ambiguous = wikidata_payload(
            wikidata_search_binding(
                0,
                "Q1",
                "Example",
                description="American software company",
            ),
            wikidata_search_binding(
                0,
                "Q2",
                "Example Inc.",
                description="European software company",
            ),
        )
        unrelated = wikidata_payload(
            wikidata_search_binding(
                0,
                "Q3",
                "Example Annual Report",
                description="annual report",
            )
        )

        self.assertIsNone(
            catalog.parse_wikidata_entity_search_profile(
                ambiguous,
                "Example Inc.",
                "EXM",
                "Nasdaq",
            )
        )
        self.assertIsNone(
            catalog.parse_wikidata_entity_search_profile(
                unrelated,
                "Example Inc.",
                "EXM",
                "Nasdaq",
            )
        )

    def test_entity_search_rejects_malformed_results(self) -> None:
        invalid_ordinal = wikidata_search_binding(
            0,
            "Q1",
            "Example",
            description="American software company",
        )
        invalid_ordinal["ordinal"]["value"] = "first"
        conflicting_label = wikidata_payload(
            wikidata_search_binding(
                0,
                "Q2",
                "Example",
                description="American software company",
            ),
            wikidata_search_binding(
                0,
                "Q2",
                "Different",
                description="American software company",
            ),
        )
        unsafe_product = wikidata_search_binding(
            0,
            "Q3",
            "Example",
            description="American software company",
            product="unsafe\u202eproduct",
        )
        truncated_payload = json.dumps(
            {
                "results": {
                    "bindings": [
                        {}
                        for _ in range(
                            catalog.MAX_WIKIDATA_ENTITY_SEARCH_ROWS + 1
                        )
                    ]
                }
            }
        ).encode()

        with self.assertRaisesRegex(RuntimeError, "invalid ordinal"):
            catalog.parse_wikidata_entity_search_profile(
                wikidata_payload(invalid_ordinal),
                "Example Inc.",
                "EXM",
                "Nasdaq",
            )
        with self.assertRaisesRegex(RuntimeError, "too many rows"):
            catalog.parse_wikidata_entity_search_profile(
                truncated_payload,
                "Example Inc.",
                "EXM",
                "Nasdaq",
            )
        with self.assertRaisesRegex(RuntimeError, "conflicting"):
            catalog.parse_wikidata_entity_search_profile(
                conflicting_label,
                "Example Inc.",
                "EXM",
                "Nasdaq",
            )
        with self.assertRaisesRegex(RuntimeError, "unsafe or too long"):
            catalog.parse_wikidata_entity_search_profile(
                wikidata_payload(unsafe_product),
                "Example Inc.",
                "EXM",
                "Nasdaq",
            )

    def test_entity_search_profile_is_stored_and_refresh_bypasses_cache(
        self,
    ) -> None:
        exact_payload = wikidata_payload(
            wikidata_binding(
                "1018724",
                "Q133848906",
                "Amazon.com, Inc.",
                description="company incorporated in Delaware",
            )
        )
        search_payload = wikidata_payload(
            wikidata_search_binding(
                0,
                "Q3884",
                "Amazon",
                ticker="AMZN",
                description="American multinational technology company",
                industry="e-commerce",
            )
        )

        with tempfile.TemporaryDirectory() as directory:
            cache_dir = Path(directory)
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                cache_dir,
            )
            calls: list[tuple[str, bool]] = []

            def query(value: str, *, bypass_cache: bool = False) -> bytes:
                calls.append((value, bypass_cache))
                key = catalog.hashlib.sha256(
                    f"{catalog.WIKIDATA_SPARQL_URL}\0{value}".encode()
                ).hexdigest()
                is_search = 'wikibase:api "EntitySearch"' in value
                client.receipts[key] = (
                    "2026-05-01T00:00:01Z"
                    if is_search
                    else "2026-05-01T00:00:00Z"
                )
                return search_payload if is_search else exact_payload

            with mock.patch.object(client, "query", side_effect=query):
                profiles, _ = catalog.load_wikidata_company_profiles(
                    client,
                    [
                        {
                            "cik": "0001018724",
                            "name": "AMAZON COM INC",
                            "symbol": "AMZN",
                            "exchange": "Nasdaq",
                        }
                    ],
                )

            self.assertEqual(profiles["0001018724"].item_id, "Q3884")
            self.assertEqual(
                profiles["0001018724"].retrieved_at,
                "2026-05-01T00:00:01Z",
            )
            self.assertEqual(len(calls), 2)
            self.assertTrue(all(not bypass for _, bypass in calls))

            cached_client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                cache_dir,
            )
            with mock.patch.object(
                cached_client,
                "query",
                side_effect=AssertionError("stored profile must be reused"),
            ):
                cached_profiles, _ = catalog.load_wikidata_company_profiles(
                    cached_client,
                    [
                        {
                            "cik": "0001018724",
                            "name": "AMAZON COM INC",
                            "symbol": "AMZN",
                            "exchange": "Nasdaq",
                        }
                    ],
                )
            self.assertEqual(cached_profiles["0001018724"].item_id, "Q3884")

            calls.clear()
            with mock.patch.object(client, "query", side_effect=query):
                refreshed_profiles, _ = catalog.load_wikidata_company_profiles(
                    client,
                    [
                        {
                            "cik": "0001018724",
                            "name": "AMAZON COM INC",
                            "symbol": "AMZN",
                            "exchange": "Nasdaq",
                        }
                    ],
                    refresh=True,
                )
            self.assertEqual(refreshed_profiles["0001018724"].item_id, "Q3884")
            self.assertEqual(len(calls), 2)
            self.assertTrue(all(bypass for _, bypass in calls))

    def test_rejects_unsafe_or_unexpected_source_values(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "unsafe or too long"):
            catalog.parse_wikidata_company_profiles(
                wikidata_payload(
                    wikidata_binding(
                        "123",
                        "Q1",
                        "Example",
                        description="unsafe\u202edescription",
                    )
                ),
                {"0000000123": "Example"},
            )
        with self.assertRaisesRegex(RuntimeError, "unexpected SEC CIK"):
            catalog.parse_wikidata_company_profiles(
                wikidata_payload(
                    wikidata_binding(
                        "999",
                        "Q1",
                        "Unexpected",
                        description="unexpected company",
                    )
                ),
                {"0000000123": "Example"},
            )

    def test_enrichment_adds_runtime_fields_and_audit_provenance(self) -> None:
        companies = [
            {
                "cik": "0000000123",
                "provenance": {},
            },
            {
                "cik": "0000000456",
                "provenance": {},
            },
        ]
        profile = catalog.CompanyProfile(
            description="American software company.",
            source_url="https://www.wikidata.org/wiki/Q2",
            item_id="Q2",
            item_label="Example",
            industries=("software industry",),
            products=("terminal software",),
        )

        catalog.enrich_companies_with_profiles(
            companies, {"0000000123": profile}
        )

        self.assertEqual(
            companies[0]["company_description"],
            "American software company.",
        )
        self.assertEqual(companies[0]["description_source"], "wikidata")
        self.assertEqual(
            companies[0]["provenance"]["company_description"]["license"],
            "CC0-1.0",
        )
        self.assertEqual(
            companies[0]["provenance"]["company_description"]["products"],
            ["terminal software"],
        )
        self.assertNotIn("company_description", companies[1])

    def test_profile_store_reuses_positive_and_negative_entries_offline(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            cache_dir = Path(directory)
            profile = catalog.CompanyProfile(
                description="American software company.",
                source_url="https://www.wikidata.org/wiki/Q2",
                item_id="Q2",
                item_label="Example",
                industries=("software",),
                retrieved_at="2026-01-01T00:00:00Z",
            )
            entries = {
                "0000000123": catalog.CompanyProfileStoreEntry(
                    issuer_name="Example Inc.",
                    issuer_key="example",
                    retrieved_at="2026-01-01T00:00:00Z",
                    last_checked_at="2026-01-02T00:00:00Z",
                    algorithm_version=(
                        catalog.WIKIDATA_PROFILE_ALGORITHM_VERSION
                    ),
                    profile=profile,
                ),
                "0000000456": catalog.CompanyProfileStoreEntry(
                    issuer_name="No Profile Corp.",
                    issuer_key="no profile",
                    retrieved_at="2026-01-03T00:00:00Z",
                    last_checked_at="2026-01-03T00:00:00Z",
                    algorithm_version=(
                        catalog.WIKIDATA_PROFILE_ALGORITHM_VERSION
                    ),
                    profile=None,
                ),
            }
            store_path = (
                cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME
            )
            catalog.write_company_profile_store(
                store_path,
                entries,
                entries,
            )
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                cache_dir,
            )

            with mock.patch.object(
                client,
                "query",
                side_effect=AssertionError("offline cache should be sufficient"),
            ) as query:
                profiles, source = catalog.load_wikidata_company_profiles(
                    client,
                    [
                        {
                            "cik": "0000000123",
                            "name": "EXAMPLE INC",
                            "symbol": "EXM",
                            "exchange": "Nasdaq",
                        },
                        {
                            "cik": "0000000456",
                            "name": "NO PROFILE CORP",
                            "symbol": "NONE",
                            "exchange": "NYSE",
                        },
                    ],
                )

            query.assert_not_called()
            self.assertEqual(set(profiles), {"0000000123"})
            self.assertEqual(
                profiles["0000000123"].retrieved_at,
                "2026-01-01T00:00:00Z",
            )
            self.assertEqual(
                source["retrieved_at"],
                "2026-01-03T00:00:00Z",
            )

            companies = [{"cik": "0000000123", "provenance": {}}]
            catalog.enrich_companies_with_profiles(companies, profiles)
            self.assertEqual(
                companies[0]["provenance"]["company_description"][
                    "retrieved_at"
                ],
                "2026-01-01T00:00:00Z",
            )

    def test_profile_store_reads_schema_one_without_product_facts(self) -> None:
        entry = catalog.CompanyProfileStoreEntry(
            issuer_name="Example Inc.",
            issuer_key="example",
            retrieved_at="2026-01-01T00:00:00Z",
            last_checked_at="2026-01-01T00:00:00Z",
            algorithm_version=1,
            profile=catalog.CompanyProfile(
                description="American software company.",
                source_url="https://www.wikidata.org/wiki/Q2",
                item_id="Q2",
                item_label="Example",
                industries=("software",),
                products=("terminal software",),
            ),
        )
        document = json.loads(
            catalog.serialize_company_profile_store(
                {"0000000123": entry}
            )
        )
        document["schema_version"] = 1
        del document["entries"]["0000000123"]["profile"]["products"]

        parsed = catalog.parse_company_profile_store(
            json.dumps(document).encode(),
            "schema-one fixture",
        )

        self.assertEqual(parsed["0000000123"].profile.products, ())

    def test_normal_build_queries_only_materially_renamed_issuer(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            cache_dir = Path(directory)
            entries = {
                "0000000123": catalog.CompanyProfileStoreEntry(
                    issuer_name="Old Name Corp.",
                    issuer_key="old name",
                    retrieved_at="2026-01-01T00:00:00Z",
                    last_checked_at="2026-01-01T00:00:00Z",
                    algorithm_version=(
                        catalog.WIKIDATA_PROFILE_ALGORITHM_VERSION
                    ),
                    profile=None,
                ),
                "0000000456": catalog.CompanyProfileStoreEntry(
                    issuer_name="Stable Inc.",
                    issuer_key="stable",
                    retrieved_at="2026-01-01T00:00:00Z",
                    last_checked_at="2026-01-01T00:00:00Z",
                    algorithm_version=(
                        catalog.WIKIDATA_PROFILE_ALGORITHM_VERSION
                    ),
                    profile=None,
                ),
            }
            catalog.write_company_profile_store(
                cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME,
                entries,
                entries,
            )
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                cache_dir,
            )

            def query(value: str, *, bypass_cache: bool = False) -> bytes:
                self.assertFalse(bypass_cache)
                self.assertIn('"123"', value)
                self.assertNotIn('"456"', value)
                key = catalog.hashlib.sha256(
                    f"{catalog.WIKIDATA_SPARQL_URL}\0{value}".encode()
                ).hexdigest()
                client.receipts[key] = "2026-02-01T00:00:00Z"
                return wikidata_payload(
                    wikidata_binding(
                        "123",
                        "Q7",
                        "New Name",
                        description="American software company",
                    )
                )

            with mock.patch.object(
                client, "query", side_effect=query
            ) as query_mock:
                profiles, _ = catalog.load_wikidata_company_profiles(
                    client,
                    [
                        {
                            "cik": "0000000123",
                            "name": "New Name Inc.",
                            "symbol": "NEW",
                            "exchange": "Nasdaq",
                        },
                        {
                            "cik": "0000000456",
                            "name": "Stable Corporation",
                            "symbol": "STBL",
                            "exchange": "NYSE",
                        },
                    ],
                )

            query_mock.assert_called_once()
            self.assertEqual(profiles["0000000123"].item_id, "Q7")
            stored = catalog.load_company_profile_store(
                cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME
            )
            self.assertEqual(stored["0000000123"].issuer_key, "new name")
            self.assertIn("0000000456", stored)

    def test_forced_refresh_bypasses_query_cache_and_replaces_unsafe_profile(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            cache_dir = Path(directory)
            old_profile = catalog.CompanyProfile(
                description="American software company.",
                source_url="https://www.wikidata.org/wiki/Q2",
                item_id="Q2",
                item_label="Current",
                industries=("software",),
                retrieved_at="2025-01-01T00:00:00Z",
            )
            historical_profile = catalog.CompanyProfile(
                description="Former public company.",
                source_url="https://www.wikidata.org/wiki/Q9",
                item_id="Q9",
                item_label="Historical",
                industries=(),
                retrieved_at="2024-01-01T00:00:00Z",
            )
            entries = {
                "0000000123": catalog.CompanyProfileStoreEntry(
                    issuer_name="Current Inc.",
                    issuer_key="current",
                    retrieved_at="2025-01-01T00:00:00Z",
                    last_checked_at="2025-01-01T00:00:00Z",
                    algorithm_version=(
                        catalog.WIKIDATA_PROFILE_ALGORITHM_VERSION
                    ),
                    profile=old_profile,
                ),
                "0000000999": catalog.CompanyProfileStoreEntry(
                    issuer_name="Historical Inc.",
                    issuer_key="historical",
                    retrieved_at="2024-01-01T00:00:00Z",
                    last_checked_at="2024-01-01T00:00:00Z",
                    algorithm_version=1,
                    profile=historical_profile,
                ),
            }
            catalog.write_company_profile_store(
                cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME,
                entries,
                {"0000000123"},
            )
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                cache_dir,
            )

            def query(value: str, *, bypass_cache: bool = False) -> bytes:
                self.assertTrue(bypass_cache)
                key = catalog.hashlib.sha256(
                    f"{catalog.WIKIDATA_SPARQL_URL}\0{value}".encode()
                ).hexdigest()
                client.receipts[key] = "2026-03-01T00:00:00Z"
                return wikidata_payload()

            with mock.patch.object(client, "query", side_effect=query):
                profiles, source = catalog.load_wikidata_company_profiles(
                    client,
                    [
                        {
                            "cik": "0000000123",
                            "name": "Current Inc.",
                            "symbol": "CURR",
                            "exchange": "Nasdaq",
                        }
                    ],
                    refresh=True,
                )

            self.assertNotIn("0000000123", profiles)
            self.assertEqual(source["retrieved_at"], "2026-03-01T00:00:00Z")
            stored = catalog.load_company_profile_store(
                cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME
            )
            self.assertIsNone(stored["0000000123"].profile)
            self.assertEqual(
                stored["0000000123"].retrieved_at,
                "2026-03-01T00:00:00Z",
            )
            self.assertEqual(
                stored["0000000123"].last_checked_at,
                "2026-03-01T00:00:00Z",
            )
            self.assertIn("0000000999", stored)
            self.assertEqual(stored["0000000999"].algorithm_version, 1)

    def test_material_rename_replaces_an_old_profile_with_fresh_negative(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            cache_dir = Path(directory)
            old_profile = catalog.CompanyProfile(
                description="American software company.",
                source_url="https://www.wikidata.org/wiki/Q2",
                item_id="Q2",
                item_label="Old Name",
                industries=("software",),
                retrieved_at="2025-01-01T00:00:00Z",
            )
            catalog.write_company_profile_store(
                cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME,
                {
                    "0000000123": catalog.CompanyProfileStoreEntry(
                        issuer_name="Old Name Inc.",
                        issuer_key="old name",
                        retrieved_at="2025-01-01T00:00:00Z",
                        last_checked_at="2025-01-01T00:00:00Z",
                        algorithm_version=(
                            catalog.WIKIDATA_PROFILE_ALGORITHM_VERSION
                        ),
                        profile=old_profile,
                    )
                },
                {"0000000123"},
            )
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                cache_dir,
            )

            def query(value: str, *, bypass_cache: bool = False) -> bytes:
                self.assertFalse(bypass_cache)
                key = catalog.hashlib.sha256(
                    f"{catalog.WIKIDATA_SPARQL_URL}\0{value}".encode()
                ).hexdigest()
                client.receipts[key] = "2026-03-15T00:00:00Z"
                return wikidata_payload()

            with mock.patch.object(
                client, "query", side_effect=query
            ) as query_mock:
                profiles, source = catalog.load_wikidata_company_profiles(
                    client,
                    [
                        {
                            "cik": "0000000123",
                            "name": "New Name Inc.",
                            "symbol": "NEW",
                            "exchange": "Nasdaq",
                        }
                    ],
                )

            self.assertEqual(query_mock.call_count, 2)
            self.assertEqual(profiles, {})
            self.assertEqual(source["retrieved_at"], "2026-03-15T00:00:00Z")
            stored = catalog.load_company_profile_store(
                cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME
            )
            self.assertEqual(stored["0000000123"].issuer_key, "new name")
            self.assertIsNone(stored["0000000123"].profile)
            self.assertEqual(
                stored["0000000123"].retrieved_at,
                "2026-03-15T00:00:00Z",
            )

    def test_algorithm_change_does_not_relabel_an_old_profile(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            cache_dir = Path(directory)
            old_profile = catalog.CompanyProfile(
                description="Legacy generated description.",
                source_url="https://www.wikidata.org/wiki/Q2",
                item_id="Q2",
                item_label="Example",
                industries=(),
                retrieved_at="2025-01-01T00:00:00Z",
            )
            catalog.write_company_profile_store(
                cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME,
                {
                    "0000000123": catalog.CompanyProfileStoreEntry(
                        issuer_name="Example Inc.",
                        issuer_key="example",
                        retrieved_at="2025-01-01T00:00:00Z",
                        last_checked_at="2025-01-01T00:00:00Z",
                        algorithm_version=1,
                        profile=old_profile,
                    )
                },
                {"0000000123"},
            )
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                cache_dir,
            )

            def query(value: str, *, bypass_cache: bool = False) -> bytes:
                self.assertFalse(bypass_cache)
                key = catalog.hashlib.sha256(
                    f"{catalog.WIKIDATA_SPARQL_URL}\0{value}".encode()
                ).hexdigest()
                client.receipts[key] = "2026-04-01T00:00:00Z"
                return wikidata_payload()

            with (
                mock.patch.object(
                    catalog,
                    "WIKIDATA_PROFILE_ALGORITHM_VERSION",
                    2,
                ),
                mock.patch.object(client, "query", side_effect=query),
            ):
                profiles, _ = catalog.load_wikidata_company_profiles(
                    client,
                    [
                        {
                            "cik": "0000000123",
                            "name": "Example Inc.",
                            "symbol": "EXM",
                            "exchange": "Nasdaq",
                        }
                    ],
                )
                stored = catalog.load_company_profile_store(
                    cache_dir / catalog.WIKIDATA_PROFILE_STORE_FILENAME
                )

            self.assertEqual(profiles, {})
            self.assertEqual(stored["0000000123"].algorithm_version, 2)
            self.assertIsNone(stored["0000000123"].profile)

    def test_profile_store_fails_closed_for_invalid_or_oversized_data(
        self,
    ) -> None:
        with self.assertRaisesRegex(RuntimeError, "unexpected shape"):
            catalog.parse_company_profile_store(b"{}", "fixture")
        with (
            mock.patch.object(
                catalog,
                "MAX_WIKIDATA_PROFILE_STORE_BYTES",
                8,
            ),
            self.assertRaisesRegex(RuntimeError, "size limit"),
        ):
            catalog.parse_company_profile_store(b"123456789", "fixture")

    def test_client_reuses_persistent_entity_search_query_cache(self) -> None:
        payload = wikidata_payload()

        class Response:
            def __enter__(self) -> "Response":
                return self

            def __exit__(self, *_args: object) -> None:
                return None

            def read(self) -> bytes:
                return payload

        with tempfile.TemporaryDirectory() as directory:
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                Path(directory),
            )
            client.minimum_interval = 0
            query = catalog.wikidata_entity_search_query(
                "AMAZON COM INC",
                "AMZN",
                "Nasdaq",
            )
            with mock.patch(
                "urllib.request.urlopen", return_value=Response()
            ) as urlopen:
                first = client.query(query)
                second = client.query(query)
                refreshed = client.query(
                    query,
                    bypass_cache=True,
                )

            self.assertEqual(first, payload)
            self.assertEqual(second, payload)
            self.assertEqual(refreshed, payload)
            self.assertEqual(urlopen.call_count, 2)
            request = urlopen.call_args.args[0]
            self.assertEqual(
                request.get_header("User-agent"),
                "stock-tui test maintainer@example.com",
            )
            self.assertEqual(
                client.source_record()["license_url"],
                catalog.WIKIDATA_LICENSE_URL,
            )

    def test_client_retries_invalid_success_response_without_caching_it(self) -> None:
        valid_payload = wikidata_payload()

        class Response:
            def __init__(self, payload: bytes) -> None:
                self.payload = payload

            def __enter__(self) -> "Response":
                return self

            def __exit__(self, *_args: object) -> None:
                return None

            def read(self) -> bytes:
                return self.payload

        with tempfile.TemporaryDirectory() as directory:
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                Path(directory),
            )
            client.minimum_interval = 0
            with (
                mock.patch(
                    "urllib.request.urlopen",
                    side_effect=[
                        Response(b"<html>temporary error</html>"),
                        Response(valid_payload),
                    ],
                ) as urlopen,
                mock.patch("time.sleep"),
            ):
                self.assertEqual(client.query("SELECT * WHERE {}"), valid_payload)
                self.assertEqual(client.query("SELECT * WHERE {}"), valid_payload)

            self.assertEqual(urlopen.call_count, 2)

    def test_client_retries_transient_source_failure(self) -> None:
        payload = wikidata_payload()

        class Response:
            def __enter__(self) -> "Response":
                return self

            def __exit__(self, *_args: object) -> None:
                return None

            def read(self) -> bytes:
                return payload

        transient = urllib.error.HTTPError(
            catalog.WIKIDATA_SPARQL_URL,
            429,
            "rate limited",
            {"Retry-After": "0"},
            None,
        )
        with tempfile.TemporaryDirectory() as directory:
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                Path(directory),
            )
            with (
                mock.patch(
                    "urllib.request.urlopen",
                    side_effect=[transient, Response()],
                ) as urlopen,
                mock.patch("time.sleep"),
            ):
                self.assertEqual(client.query("SELECT * WHERE {}"), payload)

            self.assertEqual(urlopen.call_count, 2)

    def test_client_fails_closed_after_source_retries(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            client = catalog.WikidataClient(
                "stock-tui test maintainer@example.com",
                Path(directory),
            )
            with (
                mock.patch(
                    "urllib.request.urlopen",
                    side_effect=urllib.error.URLError("offline"),
                ) as urlopen,
                mock.patch("time.sleep"),
            ):
                with self.assertRaisesRegex(
                    RuntimeError,
                    "could not reach the Wikidata SPARQL endpoint",
                ):
                    client.query("SELECT * WHERE {}")

            self.assertEqual(urlopen.call_count, 4)
            self.assertEqual(list(Path(directory).iterdir()), [])


class CanonicalTickerTests(unittest.TestCase):
    def test_alphabet_prefers_four_character_common_ticker(self) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("GOOGL", 0),
                ticker_identity("GOOG", 1),
                ticker_identity("GOOGM", 2),
                ticker_identity("GOOGN", 3),
            ]
        )

        self.assertEqual(selected["symbol"], "GOOG")

    def test_molson_coors_prefers_reviewed_class_b_symbol(self) -> None:
        for identities in (
            [
                ticker_identity("TAP-A", 0, "NYSE"),
                ticker_identity("TAP", 1, "NYSE"),
            ],
            [
                ticker_identity("TAP", 0, "NYSE"),
                ticker_identity("TAP-A", 1, "NYSE"),
            ],
        ):
            with self.subTest(order=[identity["symbol"] for identity in identities]):
                selected = catalog.canonical_identity(identities, cik=24545)

                self.assertEqual(selected["symbol"], "TAP")

    def test_molson_coors_symbol_override_is_cik_scoped(self) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("TAP-A", 0, "NYSE"),
                ticker_identity("TAP", 1, "NYSE"),
            ],
            cik=999999,
        )

        self.assertEqual(selected["symbol"], "TAP-A")

    def test_molson_coors_override_requires_tap_to_remain_listed(self) -> None:
        selected = catalog.canonical_identity(
            [ticker_identity("TAP-A", 0, "NYSE")],
            cik=24545,
        )

        self.assertEqual(selected["symbol"], "TAP-A")

    def test_short_derivative_does_not_displace_long_common_ticker(self) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("LONGC", 0),
                ticker_identity("L-PA", 1),
                ticker_identity("L-W", 2),
            ]
        )

        self.assertEqual(selected["symbol"], "LONGC")

    def test_sibling_base_ticker_displaces_warrant(self) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("OABIW", 0),
                ticker_identity("OABI", 1),
            ]
        )

        self.assertEqual(selected["symbol"], "OABI")

    def test_unrelated_short_security_does_not_displace_common_ticker(
        self,
    ) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("CMCSA", 0),
                ticker_identity("CCZ", 1, "NYSE"),
            ]
        )

        self.assertEqual(selected["symbol"], "CMCSA")

    def test_note_family_does_not_displace_unrelated_common_ticker(self) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("AMG", 0, "NYSE"),
                ticker_identity("MGR", 1, "NYSE"),
                ticker_identity("MGRB", 2, "NYSE"),
                ticker_identity("MGRD", 3, "NYSE"),
                ticker_identity("MGRE", 4, "NYSE"),
            ]
        )

        self.assertEqual(selected["symbol"], "AMG")

    def test_preferred_series_without_common_base_preserve_sec_order(
        self,
    ) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("CHSCP", 0),
                ticker_identity("CHSCL", 1),
                ticker_identity("CHSCM", 2),
                ticker_identity("CHSCN", 3),
                ticker_identity("CHSCO", 4),
            ]
        )

        self.assertEqual(selected["symbol"], "CHSCP")

    def test_invalid_short_symbol_does_not_win_length_preference(self) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("GOODL", 0),
                ticker_identity("A$", 1),
            ]
        )

        self.assertEqual(selected["symbol"], "GOODL")

    def test_equal_length_class_symbols_preserve_sec_order(self) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("BRK-B", 0, "NYSE"),
                ticker_identity("BRK-A", 1, "NYSE"),
            ]
        )

        self.assertEqual(selected["symbol"], "BRK-B")

    def test_explicit_class_symbol_is_not_shortened_without_conversion_policy(
        self,
    ) -> None:
        selected = catalog.canonical_identity(
            [
                ticker_identity("BH-A", 0, "NYSE"),
                ticker_identity("BH", 1, "NYSE"),
            ]
        )

        self.assertEqual(selected["symbol"], "BH-A")


class SecSharesRegressionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.cases = json.loads(FIXTURE.read_text(encoding="utf-8"))["cases"]

    def test_validated_issuer_share_models(self) -> None:
        for case in self.cases:
            with self.subTest(issuer=case["issuer"]):
                fact = catalog.select_fsds_shares_fact(
                    case["cik"],
                    case["symbol"],
                    fixture_components(case),
                )
                self.assertIsNotNone(fact)
                assert fact is not None
                self.assertEqual(fact.value, case["expected_value"])
                self.assertEqual(fact.method, case["expected_method"])
                self.assertEqual(fact.confidence, case["expected_confidence"])

    def test_unknown_multiclass_issuer_is_not_summed(self) -> None:
        mastercard = next(
            case for case in self.cases if case["issuer"] == "Mastercard"
        )
        fact = catalog.select_fsds_shares_fact(
            9999999, "TEST", fixture_components(mastercard)
        )
        self.assertIsNone(fact)

    def test_frame_end_rejects_malformed_and_future_dates(self) -> None:
        build_date = date(2026, 7, 24)
        self.assertEqual(
            catalog.eligible_frame_end("2026-07-24", build_date),
            "2026-07-24",
        )
        self.assertIsNone(catalog.eligible_frame_end("2026-07-25", build_date))
        self.assertIsNone(catalog.eligible_frame_end("not-a-date", build_date))

    def test_berkshire_uses_symbol_matching_reported_equivalent(self) -> None:
        berkshire = next(
            case for case in self.cases if case["issuer"] == "Berkshire Hathaway"
        )
        fact = catalog.select_fsds_shares_fact(
            berkshire["cik"], "BRK-A", fixture_components(berkshire)
        )
        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, 1438223)
        self.assertEqual(fact.method, "fsds_reported_equivalent_class")

    def test_canonical_berkshire_ticker_matches_share_policy(self) -> None:
        berkshire = next(
            case for case in self.cases if case["issuer"] == "Berkshire Hathaway"
        )
        identity = catalog.canonical_identity(
            [
                ticker_identity("BRK-B", 0, "NYSE"),
                ticker_identity("BRK-A", 1, "NYSE"),
            ]
        )
        fact = catalog.select_fsds_shares_fact(
            berkshire["cik"], identity["symbol"], fixture_components(berkshire)
        )
        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, berkshire["expected_value"])
        self.assertEqual(fact.method, "fsds_reported_equivalent_class")

    def test_berkshire_rejects_raw_issuer_total(self) -> None:
        berkshire = next(
            case for case in self.cases if case["issuer"] == "Berkshire Hathaway"
        )
        components = fixture_components(berkshire)
        raw_total = replace(
            components[0],
            value=1_500_000_000,
            tag=catalog.COMMON_SHARES_TAG,
            quarters=0,
            segments=(),
        )
        fact = catalog.select_fsds_shares_fact(
            berkshire["cik"],
            berkshire["symbol"],
            [*components, raw_total],
        )
        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, berkshire["expected_value"])
        self.assertEqual(fact.method, "fsds_reported_equivalent_class")
        provenance = catalog.shares_provenance(fact)
        self.assertEqual(
            provenance["basis"], "filer-reported Class B equivalent"
        )
        self.assertTrue(
            provenance["policy_source"].endswith("brka-20251231.htm")
        )
        self.assertEqual(provenance["components"][0]["multiplier"], 1.0)

    def test_visa_conversion_validates_redundant_class_aggregate(self) -> None:
        visa = next(case for case in self.cases if case["issuer"] == "Visa")
        components = fixture_components(visa)
        broken = [
            replace(component, value=126_000_000)
            if catalog.segment_member(component.segments, "ClassOfStock")
            == "CommonClassB1AndB2"
            else component
            for component in components
        ]
        fact = catalog.select_fsds_shares_fact(
            visa["cik"], visa["symbol"], broken
        )
        self.assertIsNone(fact)

    def test_visa_conversion_cannot_fall_through_to_generic_basic_shares(
        self,
    ) -> None:
        visa = next(case for case in self.cases if case["issuer"] == "Visa")
        components = fixture_components(visa)
        broken = [
            replace(component, value=126_000_000)
            if catalog.segment_member(component.segments, "ClassOfStock")
            == "CommonClassB1AndB2"
            else component
            for component in components
        ]
        basic = replace(
            broken[0],
            value=1_800_000_000,
            tag=catalog.BASIC_WEIGHTED_SHARES_TAG,
            quarters=1,
            segments=(),
        )
        fact = catalog.select_fsds_shares_fact(
            visa["cik"], visa["symbol"], [*broken, basic]
        )
        self.assertIsNone(fact)

    def test_visa_conversion_policy_is_accession_versioned(self) -> None:
        visa = next(case for case in self.cases if case["issuer"] == "Visa")
        components = [
            replace(component, accession="0001403161-27-999999")
            for component in fixture_components(visa)
        ]
        fact = catalog.select_fsds_shares_fact(
            visa["cik"], visa["symbol"], components
        )
        self.assertIsNone(fact)

    def test_latest_visa_conversion_policy_includes_new_class(self) -> None:
        visa = next(case for case in self.cases if case["issuer"] == "Visa")
        base = fixture_components(visa)[0]
        values = {
            "CommonClassA": 1_704_112_694,
            "CommonClassB1": 2_180_148,
            "CommonClassB2": 486_669,
            "CommonClassB3": 60_589_871,
            "CommonClassC": 17_059_152,
        }
        components = [
            replace(
                base,
                value=value,
                end="2026-07-21",
                accession="0001403161-26-000104",
                filed="20260722",
                segments=(("ClassOfStock", member),),
            )
            for member, value in values.items()
        ]

        fact = catalog.select_fsds_shares_fact(
            visa["cik"],
            visa["symbol"],
            components,
        )

        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertAlmostEqual(fact.value, 1_867_047_259.5289, places=3)
        self.assertEqual(
            fact.component_multipliers,
            (1.0, 1.5445, 1.5014, 1.4953, 4.0),
        )

    def test_visa_does_not_fall_back_past_newer_unknown_accession(self) -> None:
        visa = next(case for case in self.cases if case["issuer"] == "Visa")
        older = fixture_components(visa)
        newer = [
            replace(
                component,
                accession="0001403161-27-999999",
                filed="20270430",
                end="2027-03-31",
            )
            for component in older
        ]
        fact = catalog.select_fsds_shares_fact(
            visa["cik"], visa["symbol"], [*older, *newer]
        )
        self.assertIsNone(fact)

    def test_visa_conversion_provenance_keeps_multipliers(self) -> None:
        visa = next(case for case in self.cases if case["issuer"] == "Visa")
        fact = catalog.select_fsds_shares_fact(
            visa["cik"], visa["symbol"], fixture_components(visa)
        )
        assert fact is not None
        provenance = catalog.shares_provenance(fact)
        self.assertEqual(provenance["basis"], "Class A equivalent")
        self.assertEqual(
            [component["multiplier"] for component in provenance["components"]],
            [1.0, 1.5475, 1.5075, 4.0],
        )

    def test_dei_cover_total_has_high_confidence(self) -> None:
        submission = {
            "accession": "0000000001-26-000001",
            "filed": "20260201",
            "form": "10-K",
        }
        component = catalog.parse_fsds_share_component(
            {
                "tag": catalog.SHARES_TAG,
                "version": "dei/2025",
                "ddate": "20260115",
                "qtrs": "0",
                "uom": "shares",
                "segments": "",
                "coreg": "",
                "value": "123456789",
            },
            submission,
            "fixture_fsds_num",
        )
        self.assertIsNotNone(component)
        assert component is not None
        fact = catalog.select_fsds_shares_fact(1, "ONE", [component])
        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, 123456789)
        self.assertEqual(fact.method, "fsds_dei_cover_total")
        self.assertEqual(fact.confidence, "high")

    def test_preferred_diluted_coregistrant_and_bad_segments_are_excluded(
        self,
    ) -> None:
        submission = {
            "accession": "0000000001-26-000001",
            "filed": "20260201",
            "form": "10-K",
        }
        base = {
            "version": "us-gaap/2025",
            "ddate": "20251231",
            "qtrs": "0",
            "uom": "shares",
            "segments": "",
            "coreg": "",
            "value": "100",
        }
        invalid_rows = [
            {**base, "tag": "PreferredStockSharesOutstanding"},
            {
                **base,
                "tag": "WeightedAverageNumberOfDilutedSharesOutstanding",
                "qtrs": "4",
            },
            {
                **base,
                "tag": catalog.COMMON_SHARES_TAG,
                "coreg": "Subsidiary",
            },
            {
                **base,
                "tag": catalog.COMMON_SHARES_TAG,
                "segments": "not-an-axis-member",
            },
            {
                **base,
                "tag": catalog.COMMON_SHARES_TAG,
                "version": "issuer-extension/2025",
            },
            {
                **base,
                "tag": catalog.SHARES_TAG,
                "version": "us-gaap/2025",
            },
        ]
        for row in invalid_rows:
            with self.subTest(tag=row["tag"], segments=row["segments"]):
                self.assertIsNone(
                    catalog.parse_fsds_share_component(
                        row, submission, "fixture_fsds_num"
                    )
                )

    def test_weighted_average_uses_form_appropriate_duration(self) -> None:
        submission = {
            "accession": "0000000001-26-000001",
            "filed": "20260201",
            "form": "10-Q",
        }
        components = []
        for quarters, value in (("3", "300"), ("1", "100")):
            component = catalog.parse_fsds_share_component(
                {
                    "tag": catalog.BASIC_WEIGHTED_SHARES_TAG,
                    "version": "us-gaap/2025",
                    "ddate": "20251231",
                    "qtrs": quarters,
                    "uom": "shares",
                    "segments": "",
                    "coreg": "",
                    "value": value,
                },
                submission,
                "fixture_fsds_num",
            )
            assert component is not None
            components.append(component)
        fact = catalog.select_fsds_shares_fact(1, "ONE", components)
        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, 100)
        self.assertEqual(fact.components[0].quarters, 1)

    def test_reviewed_equal_classes_reconcile_to_aggregate_total(self) -> None:
        alphabet = next(
            case for case in self.cases if case["issuer"] == "Alphabet"
        )
        components = fixture_components(alphabet)
        broken = [
            replace(component, value=float(component.value) + 100_000_000)
            if not component.segments
            else component
            for component in components
        ]
        fact = catalog.select_fsds_shares_fact(
            alphabet["cik"], alphabet["symbol"], broken
        )
        self.assertIsNone(fact)

    def test_reviewed_equal_class_provenance_records_assumption(self) -> None:
        alphabet = next(
            case for case in self.cases if case["issuer"] == "Alphabet"
        )
        fact = catalog.select_fsds_shares_fact(
            alphabet["cik"],
            alphabet["symbol"],
            fixture_components(alphabet),
        )
        assert fact is not None
        provenance = catalog.shares_provenance(fact)
        self.assertEqual(
            provenance["basis"], "one-to-one common-share economic equivalent"
        )
        self.assertTrue(provenance["policy_source"].startswith("https://www.sec.gov/"))
        self.assertTrue(
            all(
                component["multiplier"] == 1.0
                for component in provenance["components"]
            )
        )

    def test_reviewed_equal_classes_reject_unexpected_class(self) -> None:
        meta = next(case for case in self.cases if case["issuer"] == "Meta")
        components = fixture_components(meta)
        extra_class = replace(
            components[1],
            value=1_000_000,
            segments=(("ClassOfStock", "CommonClassD"),),
        )
        fact = catalog.select_fsds_shares_fact(
            meta["cik"], meta["symbol"], [*components, extra_class]
        )
        self.assertIsNone(fact)

    def test_equal_classes_do_not_fall_back_past_newer_unknown_class(
        self,
    ) -> None:
        meta = next(case for case in self.cases if case["issuer"] == "Meta")
        older = fixture_components(meta)
        newer = [
            replace(
                component,
                accession="0001628280-27-999999",
                filed="20270430",
                end="2027-03-31",
            )
            for component in older
        ]
        newer.append(
            replace(
                newer[1],
                value=1_000_000,
                segments=(("ClassOfStock", "CommonClassD"),),
            )
        )
        fact = catalog.select_fsds_shares_fact(
            meta["cik"], meta["symbol"], [*older, *newer]
        )
        self.assertIsNone(fact)

    def test_materially_newer_medium_fact_beats_stale_high_fact(self) -> None:
        older = catalog.SharesFact(
            value=100,
            end="2024-12-31",
            accession="older",
            filed="20250131",
            form="10-K",
            source="fixture",
            method="fsds_dei_cover_total",
            confidence="high",
            components=(),
        )
        newer = catalog.SharesFact(
            value=110,
            end="2025-12-31",
            accession="newer",
            filed="20260131",
            form="10-K",
            source="fixture",
            method="fsds_common_stock_total",
            confidence="medium",
            components=(),
        )
        self.assertEqual(
            catalog.select_preferred_shares_fact([older, newer]), newer
        )

    def test_newer_medium_point_fact_beats_ninety_day_old_high_fact(self) -> None:
        older = catalog.SharesFact(
            value=100,
            end="2025-09-30",
            accession="older",
            filed="20251030",
            form="10-Q",
            source="fixture",
            method="fsds_dei_cover_total",
            confidence="high",
            components=(),
        )
        newer = catalog.SharesFact(
            value=110,
            end="2025-12-31",
            accession="newer",
            filed="20260131",
            form="10-K",
            source="fixture",
            method="fsds_common_stock_total",
            confidence="medium",
            components=(),
        )
        self.assertEqual(
            catalog.select_preferred_shares_fact([older, newer]), newer
        )

    def test_point_fact_can_override_newer_low_weighted_fallback(self) -> None:
        point_fact = catalog.SharesFact(
            value=100,
            end="2025-09-30",
            accession="point",
            filed="20251030",
            form="10-Q",
            source="fixture",
            method="fsds_common_stock_total",
            confidence="medium",
            components=(),
        )
        weighted_fallback = catalog.SharesFact(
            value=110,
            end="2026-01-31",
            accession="weighted",
            filed="20260228",
            form="10-Q",
            source="fixture",
            method="fsds_basic_weighted_average",
            confidence="low",
            components=(),
        )
        self.assertEqual(
            catalog.select_preferred_shares_fact(
                [point_fact, weighted_fallback]
            ),
            point_fact,
        )

    def test_multiclass_policy_rejects_unqualified_frame_total(self) -> None:
        berkshire = next(
            case for case in self.cases if case["issuer"] == "Berkshire Hathaway"
        )
        fallback = catalog.select_fsds_shares_fact(
            berkshire["cik"],
            berkshire["symbol"],
            fixture_components(berkshire),
        )
        assert fallback is not None
        frame = catalog.FrameFact(
            value=2_160_000_000,
            end="2025-09-30",
            accession="frame-accession",
            frame="CY2025Q3I",
            source="fixture_frame",
        )
        merged = catalog.merge_share_facts(
            {berkshire["cik"]: frame}, {berkshire["cik"]: fallback}
        )[berkshire["cik"]]
        self.assertEqual(merged, fallback)
        self.assertEqual(merged.method, "fsds_reported_equivalent_class")

    def test_reviewed_common_frame_total_supplies_dell_shares(self) -> None:
        frame = catalog.FrameFact(
            value=649_000_000,
            end="2026-05-01",
            accession="0001571996-26-000030",
            frame="CY2026Q1I",
            source=(
                "sec_frame_us_gaap_common_stock_shares_outstanding_"
                "CY2026Q1I"
            ),
        )

        merged = catalog.merge_share_facts({}, {}, {1_571_996: frame})[
            1_571_996
        ]

        self.assertEqual(merged.value, 649_000_000)
        self.assertEqual(merged.method, "sec_frame_reviewed_common_total")
        self.assertEqual(merged.confidence, "medium")
        self.assertEqual(
            merged.basis,
            "one-to-one equal-economic Class A, B, and C common shares",
        )
        self.assertIn("dell-20260501.htm", merged.policy_source or "")

    def test_dell_policy_rejects_an_unqualified_dei_cover_total(self) -> None:
        component = catalog.ShareComponent(
            value=325_000_000,
            end="2026-05-01",
            accession="unqualified",
            filed="20260603",
            form="10-Q",
            quarters=0,
            tag=catalog.SHARES_TAG,
            taxonomy="dei/2025",
            segments=(),
            source="fixture",
        )

        self.assertIsNone(
            catalog.select_fsds_shares_fact(1_571_996, "DELL", [component])
        )

    def test_common_frame_total_rejects_unreviewed_issuer(self) -> None:
        frame = catalog.FrameFact(
            value=100_000_000,
            end="2026-05-01",
            accession="unreviewed",
            frame="CY2026Q1I",
            source="fixture",
        )

        self.assertNotIn(
            9_999_999,
            catalog.merge_share_facts({}, {}, {9_999_999: frame}),
        )

    def test_public_float_accepts_downward_correction(self) -> None:
        older = catalog.FrameFact(
            value=71_600_000_000,
            end="2024-06-28",
            accession="older",
            frame="CY2024Q2I",
            source="fixture",
        )
        corrected = catalog.FrameFact(
            value=578_182_932,
            end="2025-06-30",
            accession="corrected",
            frame="CY2025Q2I",
            source="fixture",
        )
        selected = catalog.select_frame_fact(
            [older, corrected], screen_temporal_outlier=True
        )
        self.assertEqual(selected, corrected)

    def test_public_float_rejects_isolated_upward_jump(self) -> None:
        older = catalog.FrameFact(
            value=500_000_000,
            end="2024-06-30",
            accession="older",
            frame="CY2024Q2I",
            source="fixture",
        )
        erroneous = catalog.FrameFact(
            value=100_000_000_000,
            end="2025-06-30",
            accession="erroneous",
            frame="CY2025Q2I",
            source="fixture",
        )
        selected = catalog.select_frame_fact(
            [older, erroneous], screen_temporal_outlier=True
        )
        self.assertEqual(selected, older)

    def test_public_float_uses_filer_status_for_gross_scale_errors(self) -> None:
        float_fact = catalog.FrameFact(
            value=399_708_338_000,
            end="2025-06-30",
            accession="float",
            frame="CY2025Q2I",
            source="fixture",
        )
        accelerated = catalog.SicFact(
            sic=7389,
            accession="filing",
            filed="20260309",
            form="10-K",
            accelerated_filer_status="2-ACC",
            source="fixture",
        )
        self.assertFalse(
            catalog.public_float_passes_sanity(
                1720592, float_fact, None, accelerated
            )
        )

    def test_public_float_does_not_reject_legitimate_high_share_price(self) -> None:
        float_fact = catalog.FrameFact(
            value=17_263_599_044,
            end="2025-06-30",
            accession="float",
            frame="CY2025Q2I",
            source="fixture",
        )
        large_accelerated = catalog.SicFact(
            sic=6022,
            accession="filing",
            filed="20260224",
            form="10-K",
            accelerated_filer_status="1-LAF",
            source="fixture",
        )
        self.assertTrue(
            catalog.public_float_passes_sanity(
                798941, float_fact, None, large_accelerated
            )
        )

    def test_public_float_rejects_unreviewed_implied_price_scale_error(self) -> None:
        float_fact = catalog.FrameFact(
            value=4_817_275_689_000,
            end="2025-06-30",
            accession="float",
            frame="CY2025Q2I",
            source="fixture",
        )
        shares = catalog.SharesFact(
            value=49_743_567,
            end="2025-06-30",
            accession="shares",
            filed="",
            form="",
            source="fixture",
            method="sec_frame_dei_total",
            confidence="high",
            components=(),
        )
        large_accelerated = catalog.SicFact(
            sic=3825,
            accession="filing",
            filed="20260220",
            form="10-K",
            accelerated_filer_status="1-LAF",
            source="fixture",
        )
        self.assertFalse(
            catalog.public_float_passes_sanity(
                999999, float_fact, shares, large_accelerated
            )
        )

    def test_public_float_rejects_scaled_hive_fact_below_old_guard(self) -> None:
        float_fact = catalog.FrameFact(
            value=950_740_231_000,
            end="2026-03-31",
            accession="float",
            frame="CY2026Q1I",
            source="fixture",
        )
        shares = catalog.SharesFact(
            value=267_430_821,
            end="2026-05-25",
            accession="shares",
            filed="",
            form="",
            source="fixture",
            method="sec_frame_dei_total",
            confidence="high",
            components=(),
        )
        unknown_status = catalog.SicFact(
            sic=6199,
            accession="filing",
            filed="20260217",
            form="6-K",
            accelerated_filer_status="",
            source="fixture",
        )
        self.assertFalse(
            catalog.public_float_passes_sanity(
                1720424, float_fact, shares, unknown_status
            )
        )
        self.assertFalse(
            catalog.public_float_passes_sanity(
                1720424, float_fact, None, unknown_status
            )
        )

    def test_public_float_allows_reviewed_high_price_issuers(self) -> None:
        large_accelerated = catalog.SicFact(
            sic=1531,
            accession="filing",
            filed="20260220",
            form="10-K",
            accelerated_filer_status="1-LAF",
            source="fixture",
        )
        cases = (
            (906163, 20_284_815_000, 2_699_292),
            (866787, 57_759_306_497, 16_325_355),
            (1099590, 122_884_971_142, 50_697_182),
        )
        for cik, float_value, share_value in cases:
            with self.subTest(cik=cik):
                float_fact = catalog.FrameFact(
                    value=float_value,
                    end="2025-06-30",
                    accession="float",
                    frame="CY2025Q2I",
                    source="fixture",
                )
                shares = catalog.SharesFact(
                    value=share_value,
                    end="2025-06-30",
                    accession="shares",
                    filed="",
                    form="",
                    source="fixture",
                    method="sec_frame_dei_total",
                    confidence="high",
                    components=(),
                )
                self.assertTrue(
                    catalog.public_float_passes_sanity(
                        cik, float_fact, shares, large_accelerated
                    )
                )
                self.assertEqual(
                    catalog.public_float_sanity_screen(cik, shares),
                    "reviewed_high_price_issuer",
                )


class FilingCoverShareTests(unittest.TestCase):
    def test_latest_inline_submission_is_selected_safely(self) -> None:
        recent = {
            "accessionNumber": [
                "0000000001-26-000001",
                "0000000001-26-000002",
                "0000000001-26-000003",
            ],
            "filingDate": ["2026-01-15", "2026-04-15", "2026-05-15"],
            "form": ["10-K", "10-Q", "8-K"],
            "primaryDocument": ["one.htm", "two.htm", "../unsafe.htm"],
            "isInlineXBRL": [1, 1, 1],
        }

        selected = catalog.select_latest_inline_submission(
            1,
            recent,
            date(2026, 7, 29),
        )

        self.assertIsNotNone(selected)
        assert selected is not None
        self.assertEqual(selected["accession"], "0000000001-26-000002")
        self.assertEqual(selected["instance"], "two_htm.xml")

    def test_cover_parser_normalizes_class_dimensions(self) -> None:
        payload = b"""<?xml version="1.0"?>
<xbrli:xbrl
  xmlns:xbrli="http://www.xbrl.org/2003/instance"
  xmlns:xbrldi="http://xbrl.org/2006/xbrldi"
  xmlns:dei="http://xbrl.sec.gov/dei/2025"
  xmlns:us-gaap="http://fasb.org/us-gaap/2025"
  xmlns:fake="https://example.com/fake">
  <xbrli:unit id="shares"><xbrli:measure>xbrli:shares</xbrli:measure></xbrli:unit>
  <xbrli:context id="a">
    <xbrli:entity>
      <xbrli:identifier scheme="http://www.sec.gov/CIK">1</xbrli:identifier>
      <xbrli:segment>
        <xbrldi:explicitMember dimension="us-gaap:StatementClassOfStockAxis">us-gaap:CommonClassAMember</xbrldi:explicitMember>
      </xbrli:segment>
    </xbrli:entity>
    <xbrli:period><xbrli:instant>2026-06-30</xbrli:instant></xbrli:period>
  </xbrli:context>
  <dei:EntityCommonStockSharesOutstanding contextRef="a" unitRef="shares">123000000</dei:EntityCommonStockSharesOutstanding>
</xbrli:xbrl>
"""
        submission = {
            "cik": 1,
            "accession": "0000000001-26-000001",
            "filed": "2026-07-15",
            "form": "10-Q",
        }

        components = catalog.parse_filing_cover_share_components(
            payload,
            submission,
            "sec_filing_xbrl_fixture",
            date(2026, 7, 29),
        )

        self.assertEqual(len(components), 1)
        self.assertEqual(
            components[0].segments,
            (("ClassOfStock", "CommonClassA"),),
        )
        self.assertEqual(components[0].value, 123_000_000)

        fake_axis = payload.replace(
            b"us-gaap:StatementClassOfStockAxis",
            b"fake:StatementClassOfStockAxis",
        )
        self.assertEqual(
            catalog.parse_filing_cover_share_components(
                fake_axis,
                submission,
                "sec_filing_xbrl_fixture",
                date(2026, 7, 29),
            ),
            [],
        )
        rebound_axis_prefix = payload.replace(
            b"</xbrli:xbrl>",
            (
                b'<fake:marker xmlns:us-gaap="https://example.com/rebound"/>'
                b"</xbrli:xbrl>"
            ),
        )
        self.assertEqual(
            catalog.parse_filing_cover_share_components(
                rebound_axis_prefix,
                submission,
                "sec_filing_xbrl_fixture",
                date(2026, 7, 29),
            ),
            [],
        )
        invalid_dei_release = payload.replace(
            b"http://xbrl.sec.gov/dei/2025",
            b"http://xbrl.sec.gov/dei/not-a-release",
        )
        self.assertEqual(
            catalog.parse_filing_cover_share_components(
                invalid_dei_release,
                submission,
                "sec_filing_xbrl_fixture",
                date(2026, 7, 29),
            ),
            [],
        )
        invalid_gaap_release = payload.replace(
            b"http://fasb.org/us-gaap/2025",
            b"http://fasb.org/us-gaap/not-a-release",
        )
        self.assertEqual(
            catalog.parse_filing_cover_share_components(
                invalid_gaap_release,
                submission,
                "sec_filing_xbrl_fixture",
                date(2026, 7, 29),
            ),
            [],
        )

        submission["cik"] = 2
        self.assertEqual(
            catalog.parse_filing_cover_share_components(
                payload,
                submission,
                "sec_filing_xbrl_fixture",
                date(2026, 7, 29),
            ),
            [],
        )

    def test_cover_parser_ignores_same_named_non_dei_fact(self) -> None:
        payload = b"""<?xml version="1.0"?>
<xbrli:xbrl
  xmlns:xbrli="http://www.xbrl.org/2003/instance"
  xmlns:fake="https://example.invalid/fake">
  <xbrli:context id="a">
    <xbrli:entity>
      <xbrli:identifier scheme="http://www.sec.gov/CIK">1</xbrli:identifier>
    </xbrli:entity>
    <xbrli:period><xbrli:instant>2026-06-30</xbrli:instant></xbrli:period>
  </xbrli:context>
  <fake:EntityCommonStockSharesOutstanding contextRef="a">123000000</fake:EntityCommonStockSharesOutstanding>
</xbrli:xbrl>
"""
        submission = {
            "cik": 1,
            "accession": "0000000001-26-000001",
            "filed": "2026-07-15",
            "form": "10-Q",
        }

        components = catalog.parse_filing_cover_share_components(
            payload,
            submission,
            "sec_filing_xbrl_fixture",
            date(2026, 7, 29),
        )

        self.assertEqual(components, [])

    def test_reviewed_policy_uses_exact_accession_scoped_filing_fact(
        self,
    ) -> None:
        payload = b"""<?xml version="1.0"?>
<xbrli:xbrl
  xmlns:xbrli="http://www.xbrl.org/2003/instance"
  xmlns:xbrldi="http://xbrl.org/2006/xbrldi"
  xmlns:dei="http://xbrl.sec.gov/dei/2025"
  xmlns:us-gaap="http://fasb.org/us-gaap/2025"
  xmlns:issuer="https://example.com/issuer/2026"
  xmlns:fake="https://example.com/fake">
  <xbrli:unit id="shares"><xbrli:measure>xbrli:shares</xbrli:measure></xbrli:unit>
  <xbrli:context id="a">
    <xbrli:entity><xbrli:identifier scheme="http://www.sec.gov/CIK">1</xbrli:identifier>
      <xbrli:segment><xbrldi:explicitMember dimension="us-gaap:ClassOfStockAxis">us-gaap:CommonClassAMember</xbrldi:explicitMember></xbrli:segment>
    </xbrli:entity>
    <xbrli:period><xbrli:instant>2026-07-15</xbrli:instant></xbrli:period>
  </xbrli:context>
  <xbrli:context id="b">
    <xbrli:entity><xbrli:identifier scheme="http://www.sec.gov/CIK">1</xbrli:identifier>
      <xbrli:segment><xbrldi:explicitMember dimension="us-gaap:ClassOfStockAxis">us-gaap:CommonClassBMember</xbrldi:explicitMember></xbrli:segment>
    </xbrli:entity>
    <xbrli:period><xbrli:instant>2026-07-15</xbrli:instant></xbrli:period>
  </xbrli:context>
  <xbrli:context id="units">
    <xbrli:entity><xbrli:identifier scheme="http://www.sec.gov/CIK">1</xbrli:identifier></xbrli:entity>
    <xbrli:period><xbrli:startDate>2026-04-01</xbrli:startDate><xbrli:endDate>2026-06-30</xbrli:endDate></xbrli:period>
  </xbrli:context>
  <dei:EntityCommonStockSharesOutstanding contextRef="a" unitRef="shares">100</dei:EntityCommonStockSharesOutstanding>
  <dei:EntityCommonStockSharesOutstanding contextRef="b" unitRef="shares">20</dei:EntityCommonStockSharesOutstanding>
  <issuer:FullyExchangedUnits contextRef="units" unitRef="shares">40</issuer:FullyExchangedUnits>
</xbrli:xbrl>
"""
        accession = "0000000001-26-000001"
        selector = {
            "accession": accession,
            "tag": "FullyExchangedUnits",
            "namespace": "https://example.com/issuer/2026",
            "unit": "shares",
            "quarters": 1,
            "start": "2026-04-01",
            "end": "2026-06-30",
            "segments": (),
            "qualified_segments": (),
            "multiplier": 1.0,
        }
        policy = {
            "symbol": "ONE",
            "confidence": "low",
            "basis": "reviewed filing fact",
            "price_basis": "fully_converted_canonical_symbol_proxy",
            "policy_source": (
                "https://www.sec.gov/Archives/edgar/data/1/"
                "000000000126000001/one.htm"
            ),
            "members": {"CommonClassA": 1.0, "CommonClassB": 0.0},
            "filing_facts": [selector],
        }
        submission = {
            "cik": 1,
            "accession": accession,
            "filed": "2026-07-16",
            "form": "10-Q",
        }

        components = catalog.parse_filing_cover_share_components(
            payload,
            submission,
            "sec_filing_xbrl_fixture",
            date(2026, 7, 29),
            policy,
        )
        fact = catalog.select_fsds_shares_fact(
            1,
            "ONE",
            components,
            {1: policy},
        )

        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, 140)
        self.assertEqual(fact.end, "2026-06-30")
        self.assertEqual(fact.method, "filing_reviewed_fact_policy")
        self.assertEqual(fact.component_multipliers, (1.0, 0.0, 1.0))
        self.assertEqual(
            catalog.select_preferred_shares_fact([fact]),
            fact,
        )
        drifted_components = [
            replace(component, accession="0000000001-27-999999")
            for component in components
        ]
        self.assertIsNone(
            catalog.select_fsds_shares_fact(
                1,
                "ONE",
                drifted_components,
                {1: policy},
            )
        )
        duplicate_selector_policy = {
            **policy,
            "filing_facts": [selector, dict(selector)],
        }
        self.assertIsNone(
            catalog.select_fsds_shares_fact(
                1,
                "ONE",
                components,
                {1: duplicate_selector_policy},
            )
        )

        wrong_accession = {
            **policy,
            "filing_facts": [
                {**selector, "accession": "0000000001-26-000002"}
            ],
        }
        without_reported_fact = catalog.parse_filing_cover_share_components(
            payload,
            submission,
            "sec_filing_xbrl_fixture",
            date(2026, 7, 29),
            wrong_accession,
        )
        self.assertIsNone(
            catalog.select_fsds_shares_fact(
                1,
                "ONE",
                without_reported_fact,
                {1: wrong_accession},
            )
        )

        fake_unit = payload.replace(b"xbrli:shares", b"fake:shares")
        without_valid_unit = catalog.parse_filing_cover_share_components(
            fake_unit,
            submission,
            "sec_filing_xbrl_fixture",
            date(2026, 7, 29),
            policy,
        )
        self.assertIsNone(
            catalog.select_fsds_shares_fact(
                1,
                "ONE",
                without_valid_unit,
                {1: policy},
            )
        )

    def test_inconsistent_duplicate_cover_totals_fail_closed(self) -> None:
        first = catalog.ShareComponent(
            value=100,
            end="2026-06-30",
            accession="cover",
            filed="20260715",
            form="10-Q",
            quarters=0,
            tag=catalog.SHARES_TAG,
            taxonomy="dei/filing",
            segments=(),
            source="sec_filing_xbrl_cover",
        )

        self.assertIsNone(
            catalog.select_fsds_shares_fact(
                1,
                "ONE",
                [first, replace(first, value=200)],
            )
        )

    def test_duplicate_context_and_unit_ids_fail_closed(self) -> None:
        payload = b"""<?xml version="1.0"?>
<xbrli:xbrl
  xmlns:xbrli="http://www.xbrl.org/2003/instance"
  xmlns:dei="http://xbrl.sec.gov/dei/2025">
  <xbrli:unit id="shares"><xbrli:measure>xbrli:shares</xbrli:measure></xbrli:unit>
  <xbrli:context id="current">
    <xbrli:entity><xbrli:identifier scheme="http://www.sec.gov/CIK">1</xbrli:identifier></xbrli:entity>
    <xbrli:period><xbrli:instant>2026-06-30</xbrli:instant></xbrli:period>
  </xbrli:context>
  <dei:EntityCommonStockSharesOutstanding contextRef="current" unitRef="shares">100</dei:EntityCommonStockSharesOutstanding>
</xbrli:xbrl>
"""
        submission = {
            "cik": 1,
            "accession": "0000000001-26-000001",
            "filed": "2026-07-15",
            "form": "10-Q",
        }
        duplicate_unit = payload.replace(
            b'<xbrli:context id="current">',
            (
                b'<xbrli:unit id="shares">'
                b"<xbrli:measure>xbrli:shares</xbrli:measure>"
                b"</xbrli:unit>"
                b'<xbrli:context id="current">'
            ),
        )
        duplicate_context = payload.replace(
            b"<dei:EntityCommonStockSharesOutstanding",
            (
                b'<xbrli:context id="current"/>'
                b"<dei:EntityCommonStockSharesOutstanding"
            ),
        )

        for malformed in (duplicate_unit, duplicate_context):
            with self.subTest(kind=malformed is duplicate_unit):
                self.assertEqual(
                    catalog.parse_filing_cover_share_components(
                        malformed,
                        submission,
                        "sec_filing_xbrl_fixture",
                        date(2026, 7, 29),
                    ),
                    [],
                )

    def test_reviewed_equal_classes_reject_inconsistent_cover_totals(
        self,
    ) -> None:
        base = catalog.ShareComponent(
            value=100,
            end="2026-06-30",
            accession="cover",
            filed="20260715",
            form="10-Q",
            quarters=0,
            tag=catalog.SHARES_TAG,
            taxonomy="dei/filing",
            segments=(("ClassOfStock", "CommonClassA"),),
            source="sec_filing_xbrl_cover",
        )
        components = [
            base,
            replace(
                base,
                value=50,
                segments=(("ClassOfStock", "CommonClassB"),),
            ),
            replace(base, value=150, segments=()),
            replace(base, value=999, segments=()),
        ]

        self.assertIsNone(
            catalog.select_fsds_shares_fact(
                320187,
                "NKE",
                components,
            )
        )

    def test_single_common_cover_class_ignores_preferred_class(self) -> None:
        common = catalog.ShareComponent(
            value=200_000_000,
            end="2026-06-30",
            accession="cover",
            filed="20260715",
            form="10-Q",
            quarters=0,
            tag=catalog.SHARES_TAG,
            taxonomy="dei/filing",
            segments=(("ClassOfStock", "CommonUnits"),),
            source="sec_filing_xbrl_cover",
        )
        preferred = replace(
            common,
            value=10_000_000,
            segments=(("ClassOfStock", "ConvertiblePreferredUnits"),),
        )

        fact = catalog.select_fsds_shares_fact(
            1,
            "ONE",
            [common, preferred],
        )

        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, 200_000_000)
        self.assertEqual(fact.method, "filing_cover_single_class")

    def test_reviewed_cover_policy_sums_and_excludes_exact_members(self) -> None:
        base = catalog.ShareComponent(
            value=100,
            end="2026-06-30",
            accession="cover",
            filed="20260715",
            form="10-Q",
            quarters=0,
            tag=catalog.SHARES_TAG,
            taxonomy="dei/filing",
            segments=(("ClassOfStock", "CommonClassA"),),
            source="sec_filing_xbrl_cover",
        )
        components = [
            base,
            replace(
                base,
                value=50,
                segments=(("ClassOfStock", "CommonClassB"),),
            ),
        ]
        policy = {
            1: {
                "symbol": "ONE",
                "confidence": "medium",
                "basis": "reviewed fixture",
                "price_basis": "canonical_symbol_proxy",
                "policy_source": "https://www.sec.gov/fixture",
                "members": {"CommonClassA": 1.0, "CommonClassB": 0.0},
            }
        }

        fact = catalog.select_fsds_shares_fact(
            1,
            "ONE",
            components,
            policy,
        )

        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, 100)
        self.assertEqual(fact.method, "filing_cover_reviewed_policy")
        self.assertEqual(fact.component_multipliers, (1.0, 0.0))

        unexpected = replace(
            base,
            value=25,
            segments=(("ClassOfStock", "CommonClassC"),),
        )
        self.assertIsNone(
            catalog.select_fsds_shares_fact(
                1,
                "ONE",
                [*components, unexpected],
                policy,
            )
        )

    def test_reviewed_cover_policy_rejects_an_unknown_dimension(self) -> None:
        component = catalog.ShareComponent(
            value=100,
            end="2026-06-30",
            accession="cover",
            filed="20260715",
            form="10-Q",
            quarters=0,
            tag=catalog.SHARES_TAG,
            taxonomy="dei/filing",
            segments=(
                ("ClassOfStock", "CommonClassA"),
                ("Unexpected", "Unexpected"),
            ),
            source="sec_filing_xbrl_cover",
        )
        policy = {
            1: {
                "symbol": "ONE",
                "confidence": "medium",
                "basis": "reviewed fixture",
                "price_basis": "canonical_symbol_proxy",
                "policy_source": "https://www.sec.gov/fixture",
                "members": {"CommonClassA": 1.0},
            }
        }

        self.assertIsNone(
            catalog.select_fsds_shares_fact(1, "ONE", [component], policy)
        )

    def test_generic_cover_selection_rejects_an_unknown_dimension(self) -> None:
        component = catalog.ShareComponent(
            value=100,
            end="2026-06-30",
            accession="cover",
            filed="20260715",
            form="10-Q",
            quarters=0,
            tag=catalog.SHARES_TAG,
            taxonomy="dei/filing",
            segments=(
                ("ClassOfStock", "CommonClassA"),
                ("Unexpected", "Unexpected"),
            ),
            source="sec_filing_xbrl_cover",
        )

        self.assertIsNone(
            catalog.select_fsds_shares_fact(1, "ONE", [component])
        )

    def test_stale_share_fact_is_rejected_absolutely(self) -> None:
        stale = catalog.SharesFact(
            value=100,
            end="2023-09-30",
            accession="stale",
            filed="20231030",
            form="10-Q",
            source="fixture",
            method="sec_frame_dei_total",
            confidence="high",
            components=(),
        )

        self.assertIsNone(
            catalog.select_preferred_shares_fact(
                [stale],
                as_of=date(2026, 7, 29),
            )
        )

    def test_limited_partnership_weighted_units_are_low_confidence(self) -> None:
        component = catalog.parse_fsds_share_component(
            {
                "tag": catalog.LIMITED_PARTNERS_WEIGHTED_UNITS_TAG,
                "version": "us-gaap/2025",
                "ddate": "20251231",
                "qtrs": "4",
                "uom": "shares",
                "segments": "",
                "coreg": "",
                "value": "211667000",
            },
            {
                "accession": "0000000001-26-000001",
                "filed": "20260220",
                "form": "10-K",
            },
            "fixture",
        )

        self.assertIsNotNone(component)
        assert component is not None
        fact = catalog.select_fsds_shares_fact(1, "ONE", [component])
        self.assertIsNotNone(fact)
        assert fact is not None
        self.assertEqual(fact.value, 211_667_000)
        self.assertEqual(fact.confidence, "low")
        self.assertEqual(
            fact.method,
            "fsds_limited_partners_weighted_average",
        )

    def test_ambiguous_latest_cover_suppresses_old_partnership_fallback(
        self,
    ) -> None:
        old = catalog.ShareComponent(
            value=200_000_000,
            end="2025-12-31",
            accession="old",
            filed="20260220",
            form="10-K",
            quarters=4,
            tag=catalog.LIMITED_PARTNERS_WEIGHTED_UNITS_TAG,
            taxonomy="us-gaap/2025",
            segments=(),
            source="fixture_fsds_num",
        )
        common_a = catalog.ShareComponent(
            value=100_000_000,
            end="2026-06-30",
            accession="cover",
            filed="20260715",
            form="10-Q",
            quarters=0,
            tag=catalog.SHARES_TAG,
            taxonomy="dei/filing",
            segments=(("ClassOfStock", "CommonClassA"),),
            source="sec_filing_xbrl_cover",
        )
        common_b = replace(
            common_a,
            value=50_000_000,
            segments=(("ClassOfStock", "CommonClassB"),),
        )

        self.assertIsNone(
            catalog.select_fsds_shares_fact(
                1,
                "ONE",
                [old, common_a, common_b],
            )
        )

    def test_reviewed_policy_registry_matches_ambiguous_issuer_set(self) -> None:
        policies = catalog.load_reviewed_share_policies()
        symbols = {policy["symbol"] for policy in policies.values()}

        self.assertEqual(
            symbols,
            {
                "ADT",
                "ALIT",
                "ATRO",
                "BAM",
                "BATRA",
                "BF-A",
                "CMCSA",
                "COKE",
                "DDS",
                "DELL",
                "DKNG",
                "ERIE",
                "FFAI",
                "FHI",
                "FWONA",
                "H",
                "HLNE",
                "HSY",
                "HVII",
                "JBSS",
                "KRP",
                "LBRDA",
                "LBTYA",
                "LEN",
                "MC",
                "METC",
                "PJT",
                "PLNT",
                "PPLI",
                "RYAN",
                "SEI",
                "SPG",
                "STZ",
                "SUN",
                "TSN",
                "UHAL",
                "UPS",
                "VERX",
                "VGAS",
                "WHD",
                "WMG",
                "WSO",
                "WTTR",
                "YOU",
            },
        )

    def test_checked_in_catalog_resolves_every_reviewed_policy(self) -> None:
        policies = catalog.load_reviewed_share_policies()
        payload = json.loads(
            (Path(__file__).parents[2] / "data" / "sec_universe.json").read_text(
                encoding="utf-8"
            )
        )
        by_cik = {
            int(company["cik"]): company
            for company in payload["companies"]
        }

        for cik, policy in policies.items():
            with self.subTest(symbol=policy["symbol"]):
                company = by_cik[cik]
                self.assertEqual(company["symbol"], policy["symbol"])
                self.assertIsNotNone(company["shares_outstanding"])
                self.assertIsNotNone(company["shares_method"])
        self.assertEqual(
            payload["selection"]["share_coverage"]["top_100_unresolved"],
            0,
        )

    def test_catalog_validation_rejects_an_unresolved_top_100_company(
        self,
    ) -> None:
        companies = [
            {
                "cik": str(index + 1),
                "symbol": f"T{index:04d}",
                "sector": sector,
                "rank": rank,
                "shares_outstanding": 1,
                "sic_description": "Test Industry",
            }
            for index, (sector, rank) in enumerate(
                (sector, rank)
                for sector in catalog.SECTORS
                for rank in range(1, catalog.MIN_COMPANIES_PER_SECTOR + 1)
            )
        ]
        catalog.validate_catalog(companies)
        companies[0]["shares_outstanding"] = None

        with self.assertRaisesRegex(
            RuntimeError,
            "top-100 share coverage regression: T0000",
        ):
            catalog.validate_catalog(
                companies,
                {1: "policy_signature_changed"},
            )

    def test_catalog_validation_rejects_a_missing_sic_description(self) -> None:
        companies = [
            {
                "cik": str(index + 1),
                "symbol": f"T{index:04d}",
                "sector": sector,
                "rank": rank,
                "shares_outstanding": 1,
                "sic_description": "Test Industry",
            }
            for index, (sector, rank) in enumerate(
                (sector, rank)
                for sector in catalog.SECTORS
                for rank in range(1, catalog.MIN_COMPANIES_PER_SECTOR + 1)
            )
        ]
        companies[-1]["sic_description"] = None

        with self.assertRaisesRegex(
            RuntimeError,
            f"catalog SIC description coverage regression: {companies[-1]['symbol']}",
        ):
            catalog.validate_catalog(companies)


if __name__ == "__main__":
    unittest.main()
