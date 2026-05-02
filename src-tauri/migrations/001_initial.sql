-- Addons discovered / saved from sources (cache + history)
CREATE TABLE IF NOT EXISTS addons (
    id              TEXT PRIMARY KEY,          -- hash(source || page_url)
    source          TEXT NOT NULL,             -- 'sceneryaddons' | 'simplaza'
    title           TEXT NOT NULL,             -- raw title as displayed
    developer       TEXT,
    name            TEXT NOT NULL,
    version         TEXT,
    icao            TEXT,
    simulator       TEXT,
    page_url        TEXT NOT NULL,
    first_seen_at   TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_addons_icao ON addons(icao);
CREATE INDEX IF NOT EXISTS idx_addons_developer ON addons(developer);
CREATE INDEX IF NOT EXISTS idx_addons_source ON addons(source);

-- Installed addons (what the user actually has on disk)
CREATE TABLE IF NOT EXISTS installed_addons (
    id              TEXT PRIMARY KEY,
    addon_id        TEXT NOT NULL REFERENCES addons(id) ON DELETE CASCADE,
    install_path    TEXT NOT NULL,
    size_bytes      INTEGER,
    installed_at    TEXT NOT NULL DEFAULT (datetime('now')),
    version         TEXT
);

CREATE INDEX IF NOT EXISTS idx_installed_addon_id ON installed_addons(addon_id);

-- Download history (success or failure)
CREATE TABLE IF NOT EXISTS downloads (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    addon_id        TEXT REFERENCES addons(id) ON DELETE SET NULL,
    url             TEXT NOT NULL,
    method          TEXT NOT NULL,             -- 'torrent' | 'mirror' | 'direct'
    status          TEXT NOT NULL,             -- 'queued' | 'downloading' | 'completed' | 'failed' | 'cancelled'
    bytes_total     INTEGER,
    bytes_done      INTEGER,
    started_at      TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at     TEXT,
    error           TEXT
);

CREATE INDEX IF NOT EXISTS idx_downloads_status ON downloads(status);
CREATE INDEX IF NOT EXISTS idx_downloads_addon_id ON downloads(addon_id);

-- Structured app logs (complements file logs)
CREATE TABLE IF NOT EXISTS logs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ts              TEXT NOT NULL DEFAULT (datetime('now')),
    level           TEXT NOT NULL,             -- 'trace' | 'debug' | 'info' | 'warn' | 'error'
    category        TEXT NOT NULL,             -- 'app' | 'search' | 'download' | 'install' | 'ui'
    message         TEXT NOT NULL,
    data_json       TEXT                       -- arbitrary structured context
);

CREATE INDEX IF NOT EXISTS idx_logs_ts ON logs(ts);
CREATE INDEX IF NOT EXISTS idx_logs_level ON logs(level);

-- Key/value settings
CREATE TABLE IF NOT EXISTS settings (
    key             TEXT PRIMARY KEY,
    value           TEXT NOT NULL,
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
