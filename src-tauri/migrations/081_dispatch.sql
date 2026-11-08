-- (03/11) Etapa 7 — modos de lanzamiento y conjuntos de addons.
--
-- content_set: `enabled` = addons encendidos al guardar; `known` = todos los
-- que existían entonces (lo instalado después no lo decide el conjunto).
-- `kind`: 'user' (los guarda el usuario) o 'restore' (punto de restauración
-- automático antes de apagar addons).
CREATE TABLE IF NOT EXISTS content_set (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    kind        TEXT    NOT NULL DEFAULT 'user',
    enabled     TEXT    NOT NULL DEFAULT '[]',
    known       TEXT    NOT NULL DEFAULT '[]',
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- «Nunca apagar»: excepciones que el usuario marca.
CREATE TABLE IF NOT EXISTS dispatch_keep (
    folder_name TEXT PRIMARY KEY COLLATE NOCASE,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Tipos ICAO que declara el `aircraft.cfg` (los aviones cifrados no traen).
ALTER TABLE community_packages ADD COLUMN icao_types_json TEXT NOT NULL DEFAULT '[]';
