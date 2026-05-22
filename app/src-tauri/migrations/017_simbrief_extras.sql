-- (v1.1.0) Datos de payload/fuel del OFP de SimBrief para
-- pre-popular `flight_log` automáticamente al OUT — la misma fuente
-- que GSX usa internamente para su load planner.
--
-- · `pax_count`     — pasajeros planificados (puede haber dos campos
--                     en el OFP: `pax_count` y `pax_count_actual`;
--                     usamos `pax_count_actual` cuando exista, sino
--                     `pax_count`).
-- · `cargo_kg`      — carga útil planificada en kg (convertida desde
--                     lbs si el OFP venía en libras).
-- · `fuel_burn_kg`  — `enroute_burn + taxi` del OFP, también
--                     normalizado a kg.
-- · `units`         — `"lbs"` o `"kgs"`; lo guardamos por trazabilidad
--                     (debugging cuando los números no cuadren).

ALTER TABLE simbrief_flights ADD COLUMN pax_count    INTEGER;
ALTER TABLE simbrief_flights ADD COLUMN cargo_kg     INTEGER;
ALTER TABLE simbrief_flights ADD COLUMN fuel_burn_kg INTEGER;
ALTER TABLE simbrief_flights ADD COLUMN units        TEXT;
