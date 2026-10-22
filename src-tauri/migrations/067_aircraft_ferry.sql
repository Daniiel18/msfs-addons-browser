-- (v8.3 · Etapa 6.1) Traslados: vuelos de posicionamiento sin pasajeros.
--
-- Un avión está donde aterrizó por última vez (su último vuelo o traslado).
-- Si el OFP sale de otro aeropuerto, el prevuelo ofrece pedir un traslado
-- (`source = 'requested'`); si el usuario vuela igual sin pedirlo, al cerrar
-- el vuelo se cobra un traslado automático (`source = 'auto'`, ligado a ese
-- vuelo y regenerado en cada recálculo de Finanzas). Los traslados no van al
-- FlightBook: sólo cuestan dinero a la aerolínea (`airline_key`, la del vuelo
-- más reciente de la matrícula, la fija el recálculo).
-- status: active | cancelled (sólo los pedidos, y antes de llegar).
CREATE TABLE IF NOT EXISTS aircraft_ferry (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    registration     TEXT    NOT NULL,          -- normalizada (A-Z0-9)
    from_icao        TEXT    NOT NULL,
    to_icao          TEXT    NOT NULL,
    distance_nm      REAL    NOT NULL,
    model            TEXT,
    requested_at     TEXT    NOT NULL,
    arrives_at       TEXT    NOT NULL,
    cost_fuel        REAL    NOT NULL DEFAULT 0,
    cost_crew        REAL    NOT NULL DEFAULT 0,
    cost_maintenance REAL    NOT NULL DEFAULT 0,
    cost_fees        REAL    NOT NULL DEFAULT 0,
    cost_total       REAL    NOT NULL DEFAULT 0,
    source           TEXT    NOT NULL,
    flight_id        INTEGER,
    airline_key      TEXT,
    status           TEXT    NOT NULL DEFAULT 'active',
    cancelled_at     TEXT
);

CREATE INDEX IF NOT EXISTS idx_aircraft_ferry_reg ON aircraft_ferry (registration, requested_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_aircraft_ferry_flight
    ON aircraft_ferry (flight_id) WHERE flight_id IS NOT NULL;

-- Finanzas: costo y número de traslados por aerolínea.
ALTER TABLE airline_economy ADD COLUMN cost_ferry REAL NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN ferries INTEGER NOT NULL DEFAULT 0;
