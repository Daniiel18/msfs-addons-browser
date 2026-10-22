-- (v8.3 · Etapa 6.2b) Base de aviones: código Mode-S (icao24) → matrícula,
-- tipo y operador. Se llena con la base de OpenSky (aircraftDatabase.zip,
-- bajada en cada PC para uso personal y filtrada a aviones de transporte y
-- de aerolínea) y, para los que no están (entregados después de 2024), con
-- consultas a adsbdb (`source = 'adsbdb'`; `'none'` = adsbdb no lo conoce,
-- se reintenta pasados 30 días).
CREATE TABLE IF NOT EXISTS aircraft_db (
    icao24        TEXT PRIMARY KEY,      -- hex, minúsculas
    registration  TEXT,
    typecode      TEXT,                  -- designador ICAO (A20N, B738…)
    model         TEXT,
    operator_icao TEXT,
    operator      TEXT,
    built         TEXT,
    source        TEXT NOT NULL,         -- opensky | adsbdb | none
    fetched_at    TEXT
);

CREATE INDEX IF NOT EXISTS idx_aircraft_db_operator ON aircraft_db (operator_icao);
CREATE INDEX IF NOT EXISTS idx_aircraft_db_reg ON aircraft_db (registration);
