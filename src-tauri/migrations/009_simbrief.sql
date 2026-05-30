-- Historial de vuelos SimBrief.
--
-- La API pública de SimBrief (`/api/xml.fetcher.php?userid=N`) sólo
-- devuelve el OFP (Operational Flight Plan) **más reciente** del
-- usuario — no expone historial. Para acumular vuelos: cada vez
-- que el usuario refresca, descargamos el último OFP y lo
-- guardamos. Si su `ofp_id` es nuevo, la fila se inserta; si ya
-- existe, ON CONFLICT lo refresca.
--
-- Resultado práctico: a partir del momento en que el usuario
-- configura su pilot_id, cada vez que genere un vuelo en SimBrief
-- y abra/refresque la app, queda guardado.
CREATE TABLE IF NOT EXISTS simbrief_flights (
    ofp_id              TEXT PRIMARY KEY,         -- identificador único del OFP
    pilot_id            TEXT NOT NULL,
    flight_number       TEXT,
    callsign            TEXT,
    aircraft_icao       TEXT,
    origin_icao         TEXT NOT NULL,
    origin_name         TEXT,
    origin_lat          REAL NOT NULL,
    origin_lon          REAL NOT NULL,
    destination_icao    TEXT NOT NULL,
    destination_name    TEXT,
    destination_lat     REAL NOT NULL,
    destination_lon     REAL NOT NULL,
    route               TEXT,
    distance_nm         INTEGER,
    est_time_enroute_s  INTEGER,
    generated_at        TEXT,                     -- timestamp del OFP (de la API)
    fetched_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_simbrief_pilot ON simbrief_flights(pilot_id);
CREATE INDEX IF NOT EXISTS idx_simbrief_generated ON simbrief_flights(generated_at);
