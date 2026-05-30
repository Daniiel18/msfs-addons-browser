-- Dataset de aeropuertos derivado de OurAirports.
--
-- El módulo `airports.rs` baja el CSV completo, lo parsea, y deja una
-- fila por aeropuerto que tenga ICAO de 4 letras y `type` razonable
-- (large/medium/small_airport — descartamos heliports y closed). Esa
-- es la base que usa la vista de mapa para dibujar marcadores de
-- los sceneries que el catálogo local conoce.
--
-- Tener esto en SQLite y no en JSON-on-disk nos permite:
--   · JOIN directo con `addons` para resolver lat/lon en una query.
--   · Refrescos parciales sin reescribir un blob entero.
CREATE TABLE IF NOT EXISTS airports (
    icao        TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    latitude    REAL NOT NULL,
    longitude   REAL NOT NULL,
    iso_country TEXT,
    type        TEXT,
    municipality TEXT
);

-- Índices auxiliares por país y por tipo — útiles si en el futuro la
-- vista de mapa filtra por región o por tamaño.
CREATE INDEX IF NOT EXISTS idx_airports_country ON airports(iso_country);
CREATE INDEX IF NOT EXISTS idx_airports_type ON airports(type);
