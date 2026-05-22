-- Métricas extra para el FlightBook.
--
-- El usuario pidió que cada vuelo registre:
--   · ICAOs origen/destino (ya existían).
--   · Tiempo total y tiempo en aire.
--   · Gate de salida y de llegada.
--   · FPM al aterrizaje (vertical speed en el momento del touchdown).
--   · Velocidad máxima durante el vuelo (ground + true airspeed).
--
-- Para gates no tenemos parking BGL data — los dejamos como TEXT
-- para que el watcher pueda escribir "Position: lat,lon" como
-- fallback o un parking spot si en el futuro implementamos la
-- lectura de simvars `ATC PARKING SPOT` / `PARKING TYPE`.
ALTER TABLE flight_log ADD COLUMN landing_fpm           INTEGER;
ALTER TABLE flight_log ADD COLUMN max_ground_speed_kt   INTEGER;
ALTER TABLE flight_log ADD COLUMN max_true_airspeed_kt  INTEGER;
ALTER TABLE flight_log ADD COLUMN departure_gate        TEXT;
ALTER TABLE flight_log ADD COLUMN arrival_gate          TEXT;

-- Índice secundario por origin_icao — habilita búsquedas tipo
-- "todos los vuelos desde EDDF" sin escanear toda la tabla.
CREATE INDEX IF NOT EXISTS idx_flight_log_origin_icao ON flight_log(origin_icao);
CREATE INDEX IF NOT EXISTS idx_flight_log_destination_icao ON flight_log(destination_icao);
