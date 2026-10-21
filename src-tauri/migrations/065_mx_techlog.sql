-- (19/10) Etapa 3A — Tech Log propio de SimFleet (aviones sin Fenix/PMDG
-- TechLog). Mismo modelo que TechLog: avión con contadores, piezas con vida
-- propia, defectos con MEL, checks y registro por vuelo. "Solo registro":
-- nada de esto toca el avión en el simulador.

CREATE TABLE IF NOT EXISTS mx_airframe (
    registration  TEXT PRIMARY KEY,           -- normalizada (A-Z0-9)
    family        TEXT NOT NULL,              -- a320 | b737 | b777 | awide | generic
    variant       TEXT NOT NULL,
    engine        TEXT NOT NULL,
    title         TEXT,
    created_at    TEXT NOT NULL,
    counters      TEXT NOT NULL DEFAULT '{}', -- JSON (horas, ciclos, motores…)
    consumables   TEXT NOT NULL DEFAULT '{}', -- JSON "KEY:POS" -> nivel
    egt_margin    TEXT NOT NULL DEFAULT '[]', -- JSON [motor1, motor2] °C
    last_seen_at  TEXT
);

CREATE TABLE IF NOT EXISTS mx_component (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    registration      TEXT NOT NULL,
    key               TEXT NOT NULL,
    position          TEXT NOT NULL DEFAULT '',
    serial            INTEGER NOT NULL DEFAULT 0,
    installed_at      TEXT NOT NULL,
    basis_at_install  REAL NOT NULL DEFAULT 0,
    life              REAL NOT NULL,
    wear              REAL NOT NULL DEFAULT 0,
    status            TEXT NOT NULL DEFAULT 'Ok', -- Ok | Degraded | Failed
    defect_id         INTEGER,
    repairs           INTEGER NOT NULL DEFAULT 0,
    UNIQUE (registration, key, position)
);
CREATE INDEX IF NOT EXISTS idx_mx_component_reg ON mx_component (registration);

CREATE TABLE IF NOT EXISTS mx_defect (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    registration      TEXT NOT NULL,
    ata               INTEGER NOT NULL DEFAULT 0,
    title             TEXT NOT NULL,
    detail            TEXT,
    source            TEXT NOT NULL,              -- Life | Wear | Exceedance | Landing | Check | Consumable
    component_key     TEXT,
    position          TEXT,
    mel_id            TEXT,
    category          TEXT NOT NULL,              -- NoGo | A | B | C | D
    status            TEXT NOT NULL DEFAULT 'Open', -- Open | Deferred | Rectified
    raised_at         TEXT NOT NULL,
    raised_flight_id  INTEGER,
    deferred_until    TEXT,
    rectified_at      TEXT,
    rectified_how     TEXT
);
CREATE INDEX IF NOT EXISTS idx_mx_defect_reg ON mx_defect (registration, status);

CREATE TABLE IF NOT EXISTS mx_check (
    registration  TEXT NOT NULL,
    key           TEXT NOT NULL,
    last_done_at  TEXT NOT NULL,
    last_done_fh  REAL NOT NULL DEFAULT 0,
    last_done_fc  REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (registration, key)
);

CREATE TABLE IF NOT EXISTS mx_flight (
    flight_id           INTEGER PRIMARY KEY,      -- flight_log.id
    registration        TEXT NOT NULL,
    summary             TEXT,
    events              TEXT NOT NULL DEFAULT '[]',
    wear                TEXT NOT NULL DEFAULT '[]',
    dispatch_violation  INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS mx_servicing (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    registration  TEXT NOT NULL,
    at            TEXT NOT NULL,
    kind          TEXT NOT NULL,
    detail        TEXT,
    cost          REAL NOT NULL DEFAULT 0
);
