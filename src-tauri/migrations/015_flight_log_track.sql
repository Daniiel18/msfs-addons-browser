-- Track point-by-point de cada vuelo (v0.1.23).
--
-- Antes: `flight_log.last_position_*` guardaba **sólo el último**
-- punto observado, sobrescribiendo cada 10s. Suficiente para "está
-- volando ahora", inútil para "muéstrame la ruta que tomé".
--
-- Ahora: cada muestreo añade una fila a esta tabla. Una consulta
-- ordenada por `ts` produce la polyline real del vuelo (en lugar de
-- la great-circle aproximada que pinta `RoutesMapView` para vuelos
-- "completos"). Permite ver desvíos, holds, vectores ATC, etc.
--
-- Frecuencia de muestreo: cada ~10s desde el watcher mientras el
-- vuelo está en fases BlockOut / Airborne / Landed. Un vuelo de 1h
-- produce ~360 filas — ~30 KB de DB por vuelo, despreciable.
--
-- `ON DELETE CASCADE`: borrar la fila padre de `flight_log` borra
-- sus track points automáticamente. Sin esto, eliminar un vuelo
-- dejaría huérfana toda la polyline.

CREATE TABLE IF NOT EXISTS flight_log_track (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    flight_id   INTEGER NOT NULL REFERENCES flight_log(id) ON DELETE CASCADE,
    lat         REAL NOT NULL,
    lon         REAL NOT NULL,
    alt_ft      INTEGER,
    gs_kt       INTEGER,
    ts          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_flight_log_track_flight_id
    ON flight_log_track(flight_id);
CREATE INDEX IF NOT EXISTS idx_flight_log_track_flight_ts
    ON flight_log_track(flight_id, ts);
