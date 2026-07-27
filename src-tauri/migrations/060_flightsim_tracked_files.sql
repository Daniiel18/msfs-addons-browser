-- 060 (v6.2.60) Tracking de updates de addons descargados de flightsim.to.
--
-- Al instalar por el navegador EMBEBIDO, cada carpeta de Community instalada se
-- asocia al archivo de flightsim.to del que salió (file_id) junto a su versión y
-- fecha de actualización AL MOMENTO de instalar (baseline). El chequeo compara
-- el `updatedAt` actual del catálogo contra `known_updated`: si avanzó, hay una
-- versión nueva de verdad. Enfoque forward-only: sólo rastrea lo descargado por
-- SimFleet de aquí en adelante (para eso sí conocemos el file_id exacto).
CREATE TABLE IF NOT EXISTS flightsim_tracked_files (
    folder_name     TEXT PRIMARY KEY,
    file_id         TEXT NOT NULL,
    title           TEXT,
    known_version   TEXT,
    known_updated   TEXT,           -- ISO-8601 del updatedAt al instalar (baseline)
    link            TEXT,
    tracked_at      TEXT NOT NULL DEFAULT (datetime('now')),
    last_checked_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_flightsim_tracked_file_id
    ON flightsim_tracked_files(file_id);
