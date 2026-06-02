-- (v4.0.0 P7.9) Weather capturado POR track-sample.
--
-- Decisión Q10/Q11 del kickoff: capturar el clima durante el vuelo
-- (no consultar histórico inexistente al final). El watcher ya escribe
-- un track sample cada ~10s con todos los simvars; agregamos las
-- variables AMBIENT de SimConnect al mismo INSERT — el sim SABE el
-- clima real en la posición del avión en cada instante.
--
-- Al abrir el Weather modal de un vuelo pasado, leemos estos samples
-- y dibujamos viento/temperatura/nubes/precipitación reales del vuelo,
-- sin depender de APIs de clima histórico (que son de pago).
--
-- Para vuelos SIN estos datos (viejos / VAS imports), el modal cae a
-- Open-Meteo Archive API (gratuita, histórica desde 1940) — P7.9c.
ALTER TABLE flight_log_track ADD COLUMN wind_dir_deg INTEGER;
ALTER TABLE flight_log_track ADD COLUMN wind_speed_kt INTEGER;
ALTER TABLE flight_log_track ADD COLUMN oat_c REAL;          -- outside air temp
ALTER TABLE flight_log_track ADD COLUMN baro_hpa INTEGER;    -- QNH / sea-level pressure
ALTER TABLE flight_log_track ADD COLUMN visibility_m INTEGER;
ALTER TABLE flight_log_track ADD COLUMN precip_state INTEGER; -- 0 none, 2 rain, 4 snow (mask)
