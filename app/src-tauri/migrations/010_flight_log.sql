-- Registro de vuelos reales detectados vía SimConnect.
--
-- Complementa `simbrief_flights` (que captura *planes*); esta tabla
-- captura el vuelo que efectivamente se voló — origen, destino,
-- distancia real, tiempo de vuelo, altitud máxima.
--
-- El watcher (`simconnect_watcher.rs`) corre en tokio task; cuando
-- detecta despegue (estado Airborne) inserta una fila con
-- `ended_at = NULL`. Cuando detecta aterrizaje + parking brake
-- actualiza la misma fila con destino + tiempos. Si la app se
-- cierra a mitad de vuelo, la fila queda con `ended_at = NULL` —
-- la UI lo marca como «vuelo interrumpido».
--
-- `origin_icao` y `destination_icao` se rellenan haciendo lookup
-- contra la tabla `airports` por proximidad (<3nm, dentro de la
-- ventana razonable de runway/parking). Si el aeropuerto está
-- fuera de OurAirports (privados, helipuertos no listados), el
-- ICAO queda NULL pero las coordenadas siguen siendo válidas.
CREATE TABLE IF NOT EXISTS flight_log (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at          TEXT NOT NULL,           -- ISO-8601 UTC
    ended_at            TEXT,                    -- NULL mientras está en vuelo
    origin_lat          REAL NOT NULL,
    origin_lon          REAL NOT NULL,
    origin_icao         TEXT,
    origin_name         TEXT,
    destination_lat     REAL,
    destination_lon     REAL,
    destination_icao    TEXT,
    destination_name    TEXT,
    aircraft_title      TEXT,                    -- simvar TITLE
    aircraft_atc_type   TEXT,                    -- simvar ATC TYPE
    distance_nm         REAL,                    -- great-circle, calculado en landing
    flight_time_s       INTEGER,                 -- segundos entre started_at y ended_at
    max_altitude_ft     INTEGER,
    source              TEXT NOT NULL DEFAULT 'simconnect'
);

CREATE INDEX IF NOT EXISTS idx_flight_log_started ON flight_log(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_flight_log_ended ON flight_log(ended_at);
