#!/usr/bin/env python3
"""
ESPN Position Enrichment Script

Fetches player position eligibility from the ESPN Fantasy Baseball API and
enriches Razzball projection CSVs with granular, multi-position data.

Razzball CSVs use generic positions (e.g., "OF" for all outfielders). This
script replaces them with ESPN's specific eligibility (e.g., "LF/CF", "SS/3B").

Usage:
    python scripts/enrich_positions.py \
        --league-id 12345 \
        --espn-s2 "AAAA..." \
        --swid "{XXXX-XXXX-...}" \
        --hitters data/projections/2026/hitters.csv \
        --pitchers data/projections/2026/pitchers.csv

Dependencies: Python 3.8+ standard library only.
"""

import argparse
import csv
import json
import sys
import time
import unicodedata
from difflib import SequenceMatcher
from io import StringIO
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import HTTPError, URLError

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

ESPN_API_BASE = (
    "https://lm-api-reads.fantasy.espn.com/apis/v3/games/flb/seasons/{season}"
    "/segments/0/leagues/{league_id}"
)

# ESPN proTeamId -> abbreviation
ESPN_TEAM_MAP = {
    1: "BAL", 2: "BOS", 3: "LAA", 4: "CWS", 5: "CLE",
    6: "DET", 7: "KC",  8: "MIL", 9: "MIN", 10: "NYY",
    11: "OAK", 12: "SEA", 13: "TEX", 14: "TOR", 15: "ATL",
    16: "CHC", 17: "CIN", 18: "HOU", 19: "LAD", 20: "WSH",
    21: "NYM", 22: "PHI", 23: "PIT", 24: "STL", 25: "SD",
    26: "SF",  27: "COL", 28: "MIA", 29: "ARI", 30: "TB",
    0: "FA",
}

# Concrete position slot IDs (no meta-slots like OF/MI/CI/UTIL/P/BE/IL)
CONCRETE_SLOTS = {0, 1, 2, 3, 4, 8, 9, 10, 11, 14, 15}

# Slots relevant to hitters vs pitchers
HITTING_SLOTS = {0, 1, 2, 3, 4, 8, 9, 10, 11}  # C,1B,2B,3B,SS,LF,CF,RF,DH
PITCHING_SLOTS = {14, 15}  # SP, RP

# Slot ID -> display string (ordered by slot ID for consistent output)
SLOT_DISPLAY = {
    0: "C", 1: "1B", 2: "2B", 3: "3B", 4: "SS",
    8: "LF", 9: "CF", 10: "RF", 11: "DH",
    14: "SP", 15: "RP",
}

# Common team abbreviation aliases (Razzball -> ESPN)
TEAM_ALIASES = {
    "WAS": "WSH",
    "TBR": "TB",
    "CHW": "CWS",
    "ANA": "LAA",
    "KCR": "KC",
    "SDP": "SD",
    "SFG": "SF",
}

# Name suffixes to strip for matching
NAME_SUFFIXES = {"jr", "sr", "ii", "iii", "iv", "v"}

PAGE_SIZE = 250
REQUEST_DELAY_SECONDS = 1.0
FUZZY_THRESHOLD = 0.85


# ---------------------------------------------------------------------------
# Name normalization
# ---------------------------------------------------------------------------

def normalize_name(name):
    """Normalize a player name for matching.

    - Lowercase
    - Strip accents (é -> e, ñ -> n)
    - Strip punctuation (periods, hyphens, apostrophes)
    - Collapse whitespace
    - Strip common suffixes (jr, sr, ii, iii, iv)
    """
    # Lowercase
    name = name.lower().strip()

    # Decompose unicode and strip combining characters (accents)
    name = unicodedata.normalize("NFD", name)
    name = "".join(c for c in name if unicodedata.category(c) != "Mn")

    # Strip punctuation (including curly/smart quotes)
    name = name.replace(".", "").replace("-", " ").replace("'", "").replace("\u2018", "").replace("\u2019", "")

    # Collapse whitespace
    parts = name.split()

    # Strip trailing suffixes
    while parts and parts[-1] in NAME_SUFFIXES:
        parts.pop()

    return " ".join(parts)


def normalize_team(team):
    """Normalize a team abbreviation to ESPN's format."""
    team = team.upper().strip()
    return TEAM_ALIASES.get(team, team)


# ---------------------------------------------------------------------------
# ESPN API
# ---------------------------------------------------------------------------

def fetch_espn_players(league_id, espn_s2, swid, season=2026):
    """Fetch all players from the ESPN Fantasy API with pagination."""
    url = ESPN_API_BASE.format(season=season, league_id=league_id)
    url += "?view=kona_player_info"

    all_players = []
    offset = 0
    request_count = 0

    while True:
        filter_header = json.dumps({
            "players": {
                "limit": PAGE_SIZE,
                "offset": offset,
                "sortPercOwned": {"sortAsc": False, "sortPriority": 1},
            }
        })

        req = Request(url)
        req.add_header("X-Fantasy-Filter", filter_header)
        req.add_header("Accept", "application/json")

        if espn_s2:
            cookie = f"espn_s2={espn_s2}"
            if swid:
                cookie += f"; SWID={swid}"
            req.add_header("Cookie", cookie)

        try:
            with urlopen(req) as resp:
                data = json.loads(resp.read().decode("utf-8"))
        except HTTPError as e:
            print(f"ERROR: ESPN API returned {e.code}: {e.reason}", file=sys.stderr)
            if e.code == 401:
                print("  -> Check your espn_s2 and SWID cookies.", file=sys.stderr)
            elif e.code == 404:
                print("  -> Check your league ID and season.", file=sys.stderr)
            sys.exit(1)
        except URLError as e:
            print(f"ERROR: Could not reach ESPN API: {e.reason}", file=sys.stderr)
            sys.exit(1)

        players = data.get("players", [])
        request_count += 1
        batch_count = len(players)

        for p in players:
            entry = p.get("playerPoolEntry", p)
            player_data = entry.get("player", {})
            full_name = player_data.get("fullName", "")
            eligible_slots = player_data.get("eligibleSlots", [])
            pro_team_id = player_data.get("proTeamId", 0)
            default_pos_id = player_data.get("defaultPositionId", 0)
            espn_id = player_data.get("id", 0)

            all_players.append({
                "id": espn_id,
                "name": full_name,
                "eligible_slots": eligible_slots,
                "pro_team_id": pro_team_id,
                "default_position_id": default_pos_id,
                "team": ESPN_TEAM_MAP.get(pro_team_id, "???"),
            })

        print(f"  Fetched batch {request_count}: {batch_count} players (offset={offset})")

        if batch_count < PAGE_SIZE:
            break

        offset += PAGE_SIZE
        time.sleep(REQUEST_DELAY_SECONDS)

    print(f"  Total: {len(all_players)} players in {request_count} requests")
    return all_players


# ---------------------------------------------------------------------------
# Player matching
# ---------------------------------------------------------------------------

def build_espn_lookup(espn_players):
    """Build lookup structures for matching ESPN players by normalized name.

    Returns:
        name_index: dict mapping normalized_name -> list of ESPN player dicts
    """
    name_index = {}
    for p in espn_players:
        key = normalize_name(p["name"])
        name_index.setdefault(key, []).append(p)
    return name_index


def match_player(csv_name, csv_team, name_index):
    """Match a CSV player to an ESPN player.

    Returns the matched ESPN player dict, or None.
    """
    norm_name = normalize_name(csv_name)
    norm_team = normalize_team(csv_team)

    # Step 1: Exact match on normalized name
    candidates = name_index.get(norm_name)
    if candidates:
        if len(candidates) == 1:
            return candidates[0]
        # Step 2: Disambiguate by team
        for c in candidates:
            if c["team"] == norm_team:
                return c
        # Multiple matches but no team match — ambiguous, skip
        return None

    # Step 3: Fuzzy match
    best_match = None
    best_ratio = 0.0
    for key, players in name_index.items():
        ratio = SequenceMatcher(None, norm_name, key).ratio()
        if ratio >= FUZZY_THRESHOLD and ratio > best_ratio:
            # Check team for fuzzy matches
            for p in players:
                if p["team"] == norm_team:
                    best_match = p
                    best_ratio = ratio
                    break

    return best_match


# ---------------------------------------------------------------------------
# Position extraction
# ---------------------------------------------------------------------------

def extract_positions(eligible_slots, slot_filter):
    """Extract display position strings from eligible slots.

    Args:
        eligible_slots: list of ESPN slot IDs
        slot_filter: set of slot IDs to keep (HITTING_SLOTS or PITCHING_SLOTS)

    Returns:
        Slash-delimited position string (e.g., "SS/3B") or empty string if none.
        Positions are ordered by slot ID for consistency.
    """
    concrete = sorted(s for s in eligible_slots if s in slot_filter and s in CONCRETE_SLOTS)
    if not concrete:
        return ""
    return "/".join(SLOT_DISPLAY[s] for s in concrete)


def has_only_generic_of(eligible_slots):
    """Check if a player has generic OF (5) but no specific LF/CF/RF slots."""
    has_generic = 5 in eligible_slots
    has_specific = any(s in eligible_slots for s in (8, 9, 10))
    return has_generic and not has_specific


# ---------------------------------------------------------------------------
# CSV processing
# ---------------------------------------------------------------------------

def process_hitters(csv_path, name_index):
    """Process hitters CSV, enriching the ESPN position column.

    Returns (updated_rows, header, stats) where stats is a dict with
    match/change/unmatched counts and detail lists.
    """
    path = Path(csv_path)
    with open(path, newline="", encoding="utf-8") as f:
        content = f.read()

    reader = csv.DictReader(StringIO(content))
    fieldnames = reader.fieldnames

    if "ESPN" not in fieldnames:
        print(f"WARNING: No 'ESPN' column in {csv_path}", file=sys.stderr)
        return [], fieldnames, {"matched": 0, "unmatched": 0, "changes": [], "unmatched_players": []}

    stats = {"matched": 0, "unmatched": 0, "changes": [], "unmatched_players": [], "two_way": []}
    updated_rows = []

    for row in reader:
        name = row.get("Name", "").strip()
        team = row.get("Team", "").strip()
        old_pos = row.get("ESPN", "").strip()

        espn_player = match_player(name, team, name_index)

        if espn_player is None:
            stats["unmatched"] += 1
            stats["unmatched_players"].append(f"{name} ({team})")
            updated_rows.append(row)
            continue

        stats["matched"] += 1

        # Extract hitting positions
        new_pos = extract_positions(espn_player["eligible_slots"], HITTING_SLOTS)

        # Edge case: player has only generic OF, no specific LF/CF/RF
        if not new_pos and has_only_generic_of(espn_player["eligible_slots"]):
            new_pos = "OF"

        # If we got positions, update; otherwise keep original
        if new_pos and new_pos != old_pos:
            stats["changes"].append(f"{name}: {old_pos} -> {new_pos}")
            row["ESPN"] = new_pos
        elif not new_pos:
            # No hitting positions extracted — keep original
            pass

        # Check for two-way player
        pitching_pos = extract_positions(espn_player["eligible_slots"], PITCHING_SLOTS)
        if pitching_pos:
            stats["two_way"].append({"name": name, "hitting": new_pos or old_pos, "pitching": pitching_pos})

        updated_rows.append(row)

    return updated_rows, fieldnames, stats


def process_pitchers(csv_path, name_index):
    """Process pitchers CSV, enriching the POS column.

    Returns (updated_rows, header, stats).
    """
    path = Path(csv_path)
    with open(path, newline="", encoding="utf-8") as f:
        content = f.read()

    reader = csv.DictReader(StringIO(content))
    fieldnames = reader.fieldnames

    if "POS" not in fieldnames:
        print(f"WARNING: No 'POS' column in {csv_path}", file=sys.stderr)
        return [], fieldnames, {"matched": 0, "unmatched": 0, "changes": [], "unmatched_players": []}

    stats = {"matched": 0, "unmatched": 0, "changes": [], "unmatched_players": [], "two_way": []}
    updated_rows = []

    for row in reader:
        name = row.get("Name", "").strip()
        team = row.get("Team", "").strip()
        old_pos = row.get("POS", "").strip()

        espn_player = match_player(name, team, name_index)

        if espn_player is None:
            stats["unmatched"] += 1
            stats["unmatched_players"].append(f"{name} ({team})")
            updated_rows.append(row)
            continue

        stats["matched"] += 1

        # Extract pitching positions
        new_pos = extract_positions(espn_player["eligible_slots"], PITCHING_SLOTS)

        if new_pos and new_pos != old_pos:
            stats["changes"].append(f"{name}: {old_pos} -> {new_pos}")
            row["POS"] = new_pos

        # Check for two-way player (has hitting positions too)
        hitting_pos = extract_positions(espn_player["eligible_slots"], HITTING_SLOTS)
        if hitting_pos:
            stats["two_way"].append({"name": name, "hitting": hitting_pos, "pitching": new_pos or old_pos})

        updated_rows.append(row)

    return updated_rows, fieldnames, stats


def write_csv(csv_path, rows, fieldnames):
    """Write updated rows back to a CSV file."""
    path = Path(csv_path)
    output = StringIO()
    writer = csv.DictWriter(output, fieldnames=fieldnames, lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)

    path.write_text(output.getvalue(), encoding="utf-8")


# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------

def print_report(hitter_stats, pitcher_stats):
    """Print a summary report of the enrichment results."""
    print("\n=== ESPN Position Enrichment Report ===\n")

    for label, stats in [("Hitters", hitter_stats), ("Pitchers", pitcher_stats)]:
        total = stats["matched"] + stats["unmatched"]
        print(f"--- {label} ---")
        print(f"Matched: {stats['matched']}/{total}")
        print(f"Position changes: {len(stats['changes'])}")
        for change in stats["changes"][:20]:
            print(f"  {change}")
        if len(stats["changes"]) > 20:
            print(f"  ... and {len(stats['changes']) - 20} more")
        print()

        if stats["unmatched_players"]:
            print(f"Unmatched ({stats['unmatched']}):")
            for p in stats["unmatched_players"]:
                print(f"  - {p}")
            print()

    # Two-way players (combine from both files)
    two_way = {}
    for tw in hitter_stats.get("two_way", []):
        two_way[tw["name"]] = {"hitting": tw["hitting"], "pitching": tw["pitching"]}
    for tw in pitcher_stats.get("two_way", []):
        if tw["name"] in two_way:
            two_way[tw["name"]]["pitching"] = tw["pitching"]
        else:
            two_way[tw["name"]] = {"hitting": tw["hitting"], "pitching": tw["pitching"]}

    if two_way:
        print("--- Two-Way Players ---")
        for name, positions in sorted(two_way.items()):
            print(f"  {name}: hitters={positions['hitting']}, pitchers={positions['pitching']}")
        print()

    print("=== Done. Files updated. ===")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def parse_args(argv=None):
    """Parse command-line arguments."""
    parser = argparse.ArgumentParser(
        description="Enrich Razzball projection CSVs with ESPN position eligibility data."
    )
    parser.add_argument("--league-id", required=True, help="ESPN fantasy league ID")
    parser.add_argument("--espn-s2", required=True, help="espn_s2 cookie value")
    parser.add_argument("--swid", required=True, help="SWID cookie value")
    parser.add_argument(
        "--hitters",
        default="data/projections/2026/hitters.csv",
        help="Path to hitters CSV (default: data/projections/2026/hitters.csv)",
    )
    parser.add_argument(
        "--pitchers",
        default="data/projections/2026/pitchers.csv",
        help="Path to pitchers CSV (default: data/projections/2026/pitchers.csv)",
    )
    parser.add_argument(
        "--season",
        type=int,
        default=2026,
        help="ESPN season year (default: 2026)",
    )
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)

    # 1. Fetch ESPN players
    print("Fetching ESPN player data...")
    espn_players = fetch_espn_players(
        league_id=args.league_id,
        espn_s2=args.espn_s2,
        swid=args.swid,
        season=args.season,
    )

    # 2. Build lookup
    name_index = build_espn_lookup(espn_players)

    # 3. Process hitters
    print(f"\nProcessing hitters: {args.hitters}")
    hitter_rows, hitter_fields, hitter_stats = process_hitters(args.hitters, name_index)
    if hitter_rows:
        write_csv(args.hitters, hitter_rows, hitter_fields)

    # 4. Process pitchers
    print(f"Processing pitchers: {args.pitchers}")
    pitcher_rows, pitcher_fields, pitcher_stats = process_pitchers(args.pitchers, name_index)
    if pitcher_rows:
        write_csv(args.pitchers, pitcher_rows, pitcher_fields)

    # 5. Report
    print_report(hitter_stats, pitcher_stats)


if __name__ == "__main__":
    main()
