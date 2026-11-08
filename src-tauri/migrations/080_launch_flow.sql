-- (02/11) Etapa 6 — secuencias de arranque: apps, webs, carpetas y el
-- simulador, en orden. `aircraft` = tipos ICAO del OFP que la usan (JSON);
-- `steps` = los pasos en orden (JSON, ver `launch_flow::LaunchStep`).
CREATE TABLE IF NOT EXISTS launch_sequence (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    aircraft    TEXT    NOT NULL DEFAULT '[]',
    is_default  INTEGER NOT NULL DEFAULT 0,
    steps       TEXT    NOT NULL DEFAULT '[]',
    sort        INTEGER NOT NULL DEFAULT 0,
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);
