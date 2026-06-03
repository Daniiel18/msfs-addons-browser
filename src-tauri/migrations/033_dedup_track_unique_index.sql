-- (v3.24.0) FIX de RAÍZ: los track points venían DUPLICADOS ~5x,
-- inflando la base de datos (209 MB en el equipo del usuario).
--
-- Causa: cloud_sync::restore_snapshot (lo llama el auto-sync diario y el
-- restore de folder) hacía `INSERT OR IGNORE INTO flight_log_track
-- (flight_id, ts, ...)` PERO la tabla no tenía UNIQUE sobre
-- (flight_id, ts) — sólo la PK `id` AUTOINCREMENT — así que el OR IGNORE
-- nunca ignoraba y cada sync reinsertaba TODO el track otra vez. Tras N
-- syncs había N copias del mismo track concatenadas (de ahí las "líneas
-- rectas" que cruzaban el mapa entre copias).
--
-- 1) Deduplica: conserva la fila de MENOR id por cada (flight_id, ts).
DELETE FROM flight_log_track
WHERE id NOT IN (
    SELECT MIN(id) FROM flight_log_track GROUP BY flight_id, ts
);

-- 2) UNIQUE index → desde ahora TODOS los INSERT OR IGNORE (restore,
--    download_missing, watcher, VAS) son idempotentes por (flight_id, ts).
--    Ningún path puede volver a duplicar el track.
CREATE UNIQUE INDEX IF NOT EXISTS idx_flight_log_track_flight_ts
    ON flight_log_track(flight_id, ts);
