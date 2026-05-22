-- Métricas extra del FlightBook (v0.1.25): pasajeros, carga,
-- combustible consumido y segundos pausados.
--
-- · `passengers` y `cargo_kg` — manuales por ahora (la integración
--   con GSX viene en una release posterior). Editables vía
--   `update_flight_log_entry` desde el panel de detalle del vuelo.
--
-- · `fuel_used_kg` — automático: el watcher captura FUEL TOTAL
--   QUANTITY WEIGHT al OUT (pushback) y la diferencia al IN
--   (engine shutdown). El simvar viene en libras → convertimos a kg
--   (divide /2.2046).
--
-- · `paused_seconds` — total acumulado de tiempo con el sim pausado
--   durante este vuelo. El `finish_flight` resta este valor del
--   `flight_time_s` calculado por la diferencia raw started_at →
--   ended_at, para que el block-time refleje tiempo REAL volando
--   (no incluyendo cuando se pausa para preparar charts, comer, etc).

ALTER TABLE flight_log ADD COLUMN passengers INTEGER;
ALTER TABLE flight_log ADD COLUMN cargo_kg INTEGER;
ALTER TABLE flight_log ADD COLUMN fuel_used_kg INTEGER;
ALTER TABLE flight_log ADD COLUMN paused_seconds INTEGER NOT NULL DEFAULT 0;
