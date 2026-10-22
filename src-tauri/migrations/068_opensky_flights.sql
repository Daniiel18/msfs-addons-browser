-- (v8.3 · Etapa 6.2a) Vuelos reales de OpenSky, con la cuenta del usuario y
-- desde su propio PC (uso personal; nada pasa por un servidor).
--
-- SimFleet pide `/flights/all` por ventanas de 2 h UTC ya cerradas (hasta 2
-- días atrás si estuvo cerrado) y guarda SÓLO los vuelos cuyo indicativo es
-- de una aerolínea de Finanzas. `opensky_window` recuerda qué ventanas ya se
-- bajaron y cuántos créditos costaron (la cuota diaria se comparte).
CREATE TABLE IF NOT EXISTS opensky_window (
    begin_ts      INTEGER PRIMARY KEY,   -- Unix (s), alineado a 2 h UTC
    end_ts        INTEGER NOT NULL,
    fetched_at    TEXT    NOT NULL,
    status        TEXT    NOT NULL,      -- ok | error
    credits       INTEGER NOT NULL DEFAULT 0,
    total_flights INTEGER NOT NULL DEFAULT 0,
    kept_flights  INTEGER NOT NULL DEFAULT 0,
    remaining     INTEGER,               -- X-Rate-Limit-Remaining tras pedirla
    error         TEXT
);

CREATE TABLE IF NOT EXISTS real_flight (
    icao24        TEXT    NOT NULL,      -- código Mode-S (hex, minúsculas)
    first_seen    INTEGER NOT NULL,      -- Unix (s)
    last_seen     INTEGER NOT NULL,
    callsign      TEXT    NOT NULL,
    airline_icao  TEXT    NOT NULL,
    dep_icao      TEXT,                  -- estimado por OpenSky (puede faltar)
    arr_icao      TEXT,
    window_begin  INTEGER NOT NULL,
    PRIMARY KEY (icao24, first_seen)
);

CREATE INDEX IF NOT EXISTS idx_real_flight_airline ON real_flight (airline_icao, first_seen);
