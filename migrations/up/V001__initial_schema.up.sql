CREATE TABLE players (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    team        TEXT NOT NULL,
    positions   TEXT NOT NULL,
    player_type TEXT NOT NULL,
    UNIQUE(name, team)
);

CREATE TABLE projections (
    player_id INTEGER NOT NULL REFERENCES players(id),
    source    TEXT NOT NULL,
    stat_name TEXT NOT NULL,
    value     REAL NOT NULL,
    PRIMARY KEY (player_id, source, stat_name)
);

CREATE TABLE draft_picks (
    pick_number    INTEGER NOT NULL,
    team_id        TEXT NOT NULL,
    team_name      TEXT NOT NULL,
    player_id      INTEGER REFERENCES players(id),
    espn_player_id TEXT,
    player_name    TEXT NOT NULL,
    position       TEXT NOT NULL,
    price          INTEGER NOT NULL,
    eligible_slots TEXT,
    assigned_slot  INTEGER,
    draft_id       TEXT NOT NULL DEFAULT '',
    timestamp      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (pick_number, draft_id)
);

CREATE TABLE draft_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE INDEX idx_draft_picks_draft_id ON draft_picks(draft_id);
