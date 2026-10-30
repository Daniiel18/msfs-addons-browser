-- (26/10) Pistas de cada aeropuerto (OurAirports, CC0) para la tarjeta del
-- FlightBook: cabeceras con sus coordenadas (foco en el mapa), longitud,
-- anchura, superficie y helipuertos.
CREATE TABLE IF NOT EXISTS runways (
    airport     TEXT NOT NULL,
    le_ident    TEXT,
    he_ident    TEXT,
    length_ft   INTEGER,
    width_ft    INTEGER,
    surface     TEXT,
    lighted     INTEGER NOT NULL DEFAULT 0,
    le_lat      REAL,
    le_lon      REAL,
    he_lat      REAL,
    he_lon      REAL
);
CREATE INDEX IF NOT EXISTS idx_runways_airport ON runways(airport);
