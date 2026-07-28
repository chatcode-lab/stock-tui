from __future__ import annotations

import json
import unittest
from dataclasses import replace
from datetime import date
from pathlib import Path

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


if __name__ == "__main__":
    unittest.main()
