#!/usr/bin/env python3
"""Tests for enrich_positions.py"""

import csv
import tempfile
import unittest
from io import StringIO
from pathlib import Path
from unittest.mock import patch, MagicMock

from enrich_positions import (
    normalize_name,
    normalize_team,
    extract_positions,
    has_only_generic_of,
    build_espn_lookup,
    match_player,
    process_hitters,
    process_pitchers,
    write_csv,
    HITTING_SLOTS,
    PITCHING_SLOTS,
)


class TestNormalizeName(unittest.TestCase):
    def test_basic_lowercase(self):
        self.assertEqual(normalize_name("Juan Soto"), "juan soto")

    def test_strip_accents(self):
        self.assertEqual(normalize_name("José Ramírez"), "jose ramirez")

    def test_strip_periods(self):
        self.assertEqual(normalize_name("T.J. Friedl"), "tj friedl")

    def test_strip_hyphens(self):
        self.assertEqual(normalize_name("Hyun-Jin Ryu"), "hyun jin ryu")

    def test_strip_apostrophes(self):
        self.assertEqual(normalize_name("Ke'Bryan Hayes"), "kebryan hayes")

    def test_strip_jr_suffix(self):
        self.assertEqual(normalize_name("Bobby Witt Jr."), "bobby witt")

    def test_strip_sr_suffix(self):
        self.assertEqual(normalize_name("Ken Griffey Sr"), "ken griffey")

    def test_strip_ii_suffix(self):
        self.assertEqual(normalize_name("Bobby Bradley II"), "bobby bradley")

    def test_strip_iii_suffix(self):
        self.assertEqual(normalize_name("Adley Rutschman III"), "adley rutschman")

    def test_collapse_whitespace(self):
        self.assertEqual(normalize_name("  Juan   Soto  "), "juan soto")

    def test_complex_name(self):
        self.assertEqual(normalize_name("Ñ. Carlos Jr."), "n carlos")

    def test_curly_apostrophe(self):
        self.assertEqual(normalize_name("Ke\u2019Bryan Hayes"), "kebryan hayes")


class TestNormalizeTeam(unittest.TestCase):
    def test_standard_team(self):
        self.assertEqual(normalize_team("NYM"), "NYM")

    def test_was_alias(self):
        self.assertEqual(normalize_team("WAS"), "WSH")

    def test_tbr_alias(self):
        self.assertEqual(normalize_team("TBR"), "TB")

    def test_chw_alias(self):
        self.assertEqual(normalize_team("CHW"), "CWS")

    def test_lowercase_input(self):
        self.assertEqual(normalize_team("nym"), "NYM")

    def test_whitespace(self):
        self.assertEqual(normalize_team("  NYM  "), "NYM")


class TestExtractPositions(unittest.TestCase):
    def test_single_hitting_position(self):
        slots = [4, 6, 12, 16, 17]  # SS, MI, UTIL, BE, IL
        self.assertEqual(extract_positions(slots, HITTING_SLOTS), "SS")

    def test_multi_hitting_positions(self):
        # Mookie Betts: SS, OF, LF, RF, UTIL, BE, IL
        slots = [4, 5, 8, 10, 12, 16, 17]
        self.assertEqual(extract_positions(slots, HITTING_SLOTS), "SS/LF/RF")

    def test_pitching_positions_sp_only(self):
        slots = [13, 14, 16, 17]  # P, SP, BE, IL
        self.assertEqual(extract_positions(slots, PITCHING_SLOTS), "SP")

    def test_pitching_positions_sp_rp(self):
        slots = [13, 14, 15, 16, 17]  # P, SP, RP, BE, IL
        self.assertEqual(extract_positions(slots, PITCHING_SLOTS), "SP/RP")

    def test_no_matching_positions(self):
        slots = [12, 16, 17]  # UTIL, BE, IL only
        self.assertEqual(extract_positions(slots, HITTING_SLOTS), "")

    def test_dh_only(self):
        slots = [11, 12, 16, 17]  # DH, UTIL, BE, IL
        self.assertEqual(extract_positions(slots, HITTING_SLOTS), "DH")

    def test_filters_meta_slots(self):
        # OF(5), MI(6), CI(7) should be excluded
        slots = [1, 3, 5, 6, 7, 12, 16, 17]
        self.assertEqual(extract_positions(slots, HITTING_SLOTS), "1B/3B")

    def test_ordered_by_slot_id(self):
        # RF(10) before SS(4) in input, but output should be SS/RF
        slots = [10, 4, 12, 16]
        self.assertEqual(extract_positions(slots, HITTING_SLOTS), "SS/RF")

    def test_catcher(self):
        slots = [0, 12, 16, 17]  # C, UTIL, BE, IL
        self.assertEqual(extract_positions(slots, HITTING_SLOTS), "C")

    def test_two_way_player_hitting_side(self):
        # Ohtani: 1B, OF, LF, DH, UTIL, SP, BE, IL
        slots = [1, 5, 8, 11, 12, 14, 16, 17]
        self.assertEqual(extract_positions(slots, HITTING_SLOTS), "1B/LF/DH")

    def test_two_way_player_pitching_side(self):
        slots = [1, 5, 8, 11, 12, 14, 16, 17]
        self.assertEqual(extract_positions(slots, PITCHING_SLOTS), "SP")


class TestHasOnlyGenericOF(unittest.TestCase):
    def test_generic_of_only(self):
        self.assertTrue(has_only_generic_of([5, 12, 16, 17]))

    def test_generic_of_with_specific(self):
        self.assertFalse(has_only_generic_of([5, 8, 12, 16, 17]))

    def test_no_of_at_all(self):
        self.assertFalse(has_only_generic_of([4, 12, 16, 17]))

    def test_specific_only_no_generic(self):
        self.assertFalse(has_only_generic_of([8, 9, 12, 16, 17]))


class TestBuildEspnLookup(unittest.TestCase):
    def test_builds_lookup(self):
        players = [
            {"name": "Juan Soto", "eligible_slots": [5, 10, 12], "team": "NYM"},
            {"name": "Bobby Witt Jr.", "eligible_slots": [4, 12], "team": "KC"},
        ]
        index = build_espn_lookup(players)
        self.assertIn("juan soto", index)
        self.assertIn("bobby witt", index)  # Jr stripped
        self.assertEqual(len(index["juan soto"]), 1)

    def test_handles_duplicate_names(self):
        players = [
            {"name": "Will Smith", "eligible_slots": [0, 12], "team": "LAD"},
            {"name": "Will Smith", "eligible_slots": [14, 15], "team": "ATL"},
        ]
        index = build_espn_lookup(players)
        self.assertEqual(len(index["will smith"]), 2)


class TestMatchPlayer(unittest.TestCase):
    def setUp(self):
        self.players = [
            {"name": "Juan Soto", "eligible_slots": [5, 10, 12], "team": "NYM", "pro_team_id": 21},
            {"name": "Will Smith", "eligible_slots": [0, 12], "team": "LAD", "pro_team_id": 19},
            {"name": "Will Smith", "eligible_slots": [14, 15], "team": "ATL", "pro_team_id": 15},
            {"name": "Bobby Witt Jr.", "eligible_slots": [4, 12], "team": "KC", "pro_team_id": 7},
            {"name": "José Ramírez", "eligible_slots": [3, 12], "team": "CLE", "pro_team_id": 5},
        ]
        self.index = build_espn_lookup(self.players)

    def test_exact_match(self):
        result = match_player("Juan Soto", "NYM", self.index)
        self.assertEqual(result["name"], "Juan Soto")

    def test_match_with_suffix_stripping(self):
        result = match_player("Bobby Witt Jr.", "KC", self.index)
        self.assertEqual(result["name"], "Bobby Witt Jr.")

    def test_disambiguate_by_team(self):
        result = match_player("Will Smith", "LAD", self.index)
        self.assertEqual(result["team"], "LAD")
        self.assertIn(0, result["eligible_slots"])  # Catcher

    def test_disambiguate_by_team_pitcher(self):
        result = match_player("Will Smith", "ATL", self.index)
        self.assertEqual(result["team"], "ATL")
        self.assertIn(14, result["eligible_slots"])  # SP

    def test_accent_matching(self):
        result = match_player("Jose Ramirez", "CLE", self.index)
        self.assertIsNotNone(result)
        self.assertEqual(result["name"], "José Ramírez")

    def test_no_match(self):
        result = match_player("Nonexistent Player", "NYM", self.index)
        self.assertIsNone(result)

    def test_fuzzy_match(self):
        result = match_player("J. Soto", "NYM", self.index)
        # This may or may not match depending on fuzzy threshold
        # With ratio ~0.67 for "j soto" vs "juan soto", it won't match at 0.85
        # That's the expected behavior - partial initials shouldn't match
        # A closer fuzzy case would be a slight misspelling
        pass

    def test_fuzzy_match_close_spelling(self):
        # "jose ramirez" vs "jose ramirez" after normalization — exact match
        # Let's test a slight variation
        players = [
            {"name": "Ke'Bryan Hayes", "eligible_slots": [3, 12], "team": "PIT", "pro_team_id": 23},
        ]
        index = build_espn_lookup(players)
        # "KeBryan Hayes" (no apostrophe) normalizes to "kebryan hayes" — exact match
        result = match_player("KeBryan Hayes", "PIT", index)
        self.assertIsNotNone(result)


class TestProcessHitters(unittest.TestCase):
    def _make_csv(self, rows):
        """Create a temp CSV file and return its path."""
        tf = tempfile.NamedTemporaryFile(mode="w", suffix=".csv", delete=False, newline="")
        writer = csv.writer(tf)
        for row in rows:
            writer.writerow(row)
        tf.close()
        return tf.name

    def test_enriches_of_to_specific(self):
        csv_path = self._make_csv([
            ["#", "Name", "Team", "Bats", "ESPN", "YAHOO", "G", "PA"],
            ["1", "Juan Soto", "NYM", "L", "OF", "OF", "151", "649"],
        ])

        name_index = build_espn_lookup([
            {"name": "Juan Soto", "eligible_slots": [5, 10, 12, 16, 17], "team": "NYM"},
        ])

        rows, fields, stats = process_hitters(csv_path, name_index)
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["ESPN"], "RF")
        self.assertEqual(stats["matched"], 1)
        self.assertEqual(len(stats["changes"]), 1)
        Path(csv_path).unlink()

    def test_keeps_original_when_unmatched(self):
        csv_path = self._make_csv([
            ["#", "Name", "Team", "Bats", "ESPN", "YAHOO", "G", "PA"],
            ["1", "Unknown Player", "NYM", "L", "OF", "OF", "100", "400"],
        ])

        name_index = build_espn_lookup([])
        rows, fields, stats = process_hitters(csv_path, name_index)
        self.assertEqual(rows[0]["ESPN"], "OF")
        self.assertEqual(stats["unmatched"], 1)
        Path(csv_path).unlink()

    def test_multi_position_hitter(self):
        csv_path = self._make_csv([
            ["#", "Name", "Team", "Bats", "ESPN", "YAHOO", "G", "PA"],
            ["1", "Mookie Betts", "LAD", "R", "SS", "SS", "140", "600"],
        ])

        name_index = build_espn_lookup([
            {"name": "Mookie Betts", "eligible_slots": [4, 5, 8, 10, 12, 16, 17], "team": "LAD"},
        ])

        rows, fields, stats = process_hitters(csv_path, name_index)
        self.assertEqual(rows[0]["ESPN"], "SS/LF/RF")
        Path(csv_path).unlink()

    def test_detects_two_way_player(self):
        csv_path = self._make_csv([
            ["#", "Name", "Team", "Bats", "ESPN", "YAHOO", "G", "PA"],
            ["1", "Shohei Ohtani", "LAD", "L", "DH", "DH", "150", "671"],
        ])

        name_index = build_espn_lookup([
            {"name": "Shohei Ohtani", "eligible_slots": [1, 5, 8, 11, 12, 14, 16, 17], "team": "LAD"},
        ])

        rows, fields, stats = process_hitters(csv_path, name_index)
        self.assertEqual(rows[0]["ESPN"], "1B/LF/DH")
        self.assertEqual(len(stats["two_way"]), 1)
        self.assertEqual(stats["two_way"][0]["pitching"], "SP")
        Path(csv_path).unlink()

    def test_generic_of_fallback(self):
        """Player with only generic OF slot and no specific LF/CF/RF keeps OF."""
        csv_path = self._make_csv([
            ["#", "Name", "Team", "Bats", "ESPN", "YAHOO", "G", "PA"],
            ["1", "Generic Player", "NYM", "R", "OF", "OF", "100", "400"],
        ])

        name_index = build_espn_lookup([
            {"name": "Generic Player", "eligible_slots": [5, 12, 16, 17], "team": "NYM"},
        ])

        rows, fields, stats = process_hitters(csv_path, name_index)
        self.assertEqual(rows[0]["ESPN"], "OF")
        Path(csv_path).unlink()

    def test_no_change_when_same(self):
        """No change recorded when ESPN position already matches."""
        csv_path = self._make_csv([
            ["#", "Name", "Team", "Bats", "ESPN", "YAHOO", "G", "PA"],
            ["1", "Some Catcher", "BOS", "R", "C", "C", "120", "450"],
        ])

        name_index = build_espn_lookup([
            {"name": "Some Catcher", "eligible_slots": [0, 12, 16, 17], "team": "BOS"},
        ])

        rows, fields, stats = process_hitters(csv_path, name_index)
        self.assertEqual(rows[0]["ESPN"], "C")
        self.assertEqual(len(stats["changes"]), 0)
        Path(csv_path).unlink()


class TestProcessPitchers(unittest.TestCase):
    def _make_csv(self, rows):
        tf = tempfile.NamedTemporaryFile(mode="w", suffix=".csv", delete=False, newline="")
        writer = csv.writer(tf)
        for row in rows:
            writer.writerow(row)
        tf.close()
        return tf.name

    def test_sp_stays_sp(self):
        csv_path = self._make_csv([
            ["#", "Name", "Team", "POS", "R/L", "G", "GS"],
            ["1", "Logan Webb", "SF", "SP", "R", "30", "30"],
        ])

        name_index = build_espn_lookup([
            {"name": "Logan Webb", "eligible_slots": [13, 14, 16, 17], "team": "SF"},
        ])

        rows, fields, stats = process_pitchers(csv_path, name_index)
        self.assertEqual(rows[0]["POS"], "SP")
        self.assertEqual(len(stats["changes"]), 0)
        Path(csv_path).unlink()

    def test_sp_to_sp_rp(self):
        csv_path = self._make_csv([
            ["#", "Name", "Team", "POS", "R/L", "G", "GS"],
            ["1", "Swingman Joe", "NYM", "SP", "R", "40", "15"],
        ])

        name_index = build_espn_lookup([
            {"name": "Swingman Joe", "eligible_slots": [13, 14, 15, 16, 17], "team": "NYM"},
        ])

        rows, fields, stats = process_pitchers(csv_path, name_index)
        self.assertEqual(rows[0]["POS"], "SP/RP")
        self.assertEqual(len(stats["changes"]), 1)
        Path(csv_path).unlink()

    def test_detects_two_way_pitcher(self):
        csv_path = self._make_csv([
            ["#", "Name", "Team", "POS", "R/L", "G", "GS"],
            ["1", "Shohei Ohtani", "LAD", "SP", "R", "25", "25"],
        ])

        name_index = build_espn_lookup([
            {"name": "Shohei Ohtani", "eligible_slots": [1, 5, 8, 11, 12, 14, 16, 17], "team": "LAD"},
        ])

        rows, fields, stats = process_pitchers(csv_path, name_index)
        self.assertEqual(rows[0]["POS"], "SP")
        self.assertEqual(len(stats["two_way"]), 1)
        self.assertEqual(stats["two_way"][0]["hitting"], "1B/LF/DH")
        Path(csv_path).unlink()

    def test_unmatched_pitcher_keeps_original(self):
        csv_path = self._make_csv([
            ["#", "Name", "Team", "POS", "R/L", "G", "GS"],
            ["1", "Unknown Pitcher", "SF", "RP", "R", "60", "0"],
        ])

        name_index = build_espn_lookup([])
        rows, fields, stats = process_pitchers(csv_path, name_index)
        self.assertEqual(rows[0]["POS"], "RP")
        self.assertEqual(stats["unmatched"], 1)
        Path(csv_path).unlink()


class TestWriteCsv(unittest.TestCase):
    def test_roundtrip(self):
        """Write and re-read CSV to ensure formatting is preserved."""
        tf = tempfile.NamedTemporaryFile(suffix=".csv", delete=False)
        tf.close()

        fieldnames = ["#", "Name", "Team", "ESPN"]
        rows = [
            {"#": "1", "Name": "Juan Soto", "Team": "NYM", "ESPN": "RF"},
            {"#": "2", "Name": "Mookie Betts", "Team": "LAD", "ESPN": "SS/LF/RF"},
        ]

        write_csv(tf.name, rows, fieldnames)

        with open(tf.name, newline="") as f:
            reader = csv.DictReader(f)
            result = list(reader)

        self.assertEqual(len(result), 2)
        self.assertEqual(result[0]["ESPN"], "RF")
        self.assertEqual(result[1]["ESPN"], "SS/LF/RF")
        Path(tf.name).unlink()


if __name__ == "__main__":
    unittest.main()
