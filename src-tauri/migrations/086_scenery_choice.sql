-- (05/11) Qué escenario usar en un aeropuerto con varios encendidos, o que no
-- hay que avisar en él.
CREATE TABLE IF NOT EXISTS scenery_choice (
    icao TEXT PRIMARY KEY COLLATE NOCASE,
    folder_name TEXT,
    ignored INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
