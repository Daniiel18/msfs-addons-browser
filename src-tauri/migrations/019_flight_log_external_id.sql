-- (v3.5.0 F2) `external_id` para deduplicar imports de fuentes externas
-- como VAS-ACARS, donde cada vuelo tiene un identificador estable (UUID
-- del filename) que NO depende de origin/destination/timestamp.
--
-- Sin esta columna, el importer de VAS-ACARS tendría que deduplicar por
-- la tupla (origin_icao, destination_icao, started_at, source) — frágil
-- porque (a) los timestamps pueden tener 1s de drift entre passes, y
-- (b) un usuario serio vuela SAEZ→KJFK 10× y debe ver 10 vuelos, no 1
-- dedup-eado. UUID del archivo identifica unívocamente el vuelo.
--
-- Compatibilidad: NULL para vuelos preexistentes (capturados vía
-- SimConnect, que no tienen identificador externo). El importer de VAS
-- llena este campo; futuros importers (Volanta, SimToolkit, etc.) lo
-- usarán igual.
--
-- Índice compuesto (source, external_id) para que el dedup SELECT sea
-- O(log n) en vez de full-scan. WHERE clause limita el índice a las
-- filas con external_id no-null — mantiene el índice pequeño porque
-- la mayoría de filas (SimConnect captures) lo dejan NULL.

ALTER TABLE flight_log ADD COLUMN external_id TEXT;

CREATE INDEX IF NOT EXISTS idx_flight_log_external
    ON flight_log(source, external_id)
    WHERE external_id IS NOT NULL;