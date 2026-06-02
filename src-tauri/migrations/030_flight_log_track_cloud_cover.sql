-- (v3.19.0 — P7.9c) Cobertura de nubes de la VIDA REAL capturada por
-- sample durante el vuelo, vía Open-Meteo (gratis, sin API key).
--
-- MSFS 2020 no expone la cobertura de nubes por SimConnect de forma
-- fiable (los simvars de nubes están deprecados), y el usuario quiere
-- ver las nubes REALES del momento/lugar del vuelo — no las del
-- simulador. Tomamos el dato de Open-Meteo en la posición del avión y
-- lo guardamos aquí para mostrarlo luego como histórico en el Weather
-- modal, sin depender de APIs de pago ni de tener internet al revisar.
--
-- Porcentajes 0..100. cloud_cover_pct = total; low/mid/high = capas.
ALTER TABLE flight_log_track ADD COLUMN cloud_cover_pct INTEGER;
ALTER TABLE flight_log_track ADD COLUMN cloud_low_pct INTEGER;
ALTER TABLE flight_log_track ADD COLUMN cloud_mid_pct INTEGER;
ALTER TABLE flight_log_track ADD COLUMN cloud_high_pct INTEGER;
