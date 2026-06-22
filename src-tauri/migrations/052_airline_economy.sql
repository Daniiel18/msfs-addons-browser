-- (v6.1) Economía de aerolíneas.
--
-- Modelo: cada aerolínea tiene un VALOR BASE realista (su "banco", real o
-- estimado en miles de millones) y un ledger acumulado de ingresos/costes que
-- se va actualizando con cada vuelo. El P&L por vuelo se cachea en `flight_pnl`
-- (idempotente: una fila por vuelo) para no recalcular ni doblar conteos, y el
-- ledger por aerolínea se agrega desde ahí.

CREATE TABLE IF NOT EXISTS airline_economy (
    airline_key      TEXT PRIMARY KEY,        -- ICAO o nombre normalizado (UPPER)
    airline_icao     TEXT,
    airline_name     TEXT,
    is_lowcost       INTEGER NOT NULL DEFAULT 0,
    start_valuation  REAL    NOT NULL,        -- valor base USD (real o estimado)
    revenue          REAL    NOT NULL DEFAULT 0,
    costs            REAL    NOT NULL DEFAULT 0,
    flights          INTEGER NOT NULL DEFAULT 0,
    updated_at       TEXT
);

CREATE TABLE IF NOT EXISTS flight_pnl (
    flight_id         INTEGER PRIMARY KEY,    -- = flight_log.id
    airline_key       TEXT    NOT NULL,
    pax               INTEGER,
    revenue_tickets   REAL    NOT NULL DEFAULT 0,
    revenue_ancillary REAL    NOT NULL DEFAULT 0,  -- snacks, equipaje (LCC), extras
    cost_fuel         REAL    NOT NULL DEFAULT 0,
    cost_catering     REAL    NOT NULL DEFAULT 0,
    cost_handling     REAL    NOT NULL DEFAULT 0,
    cost_maintenance  REAL    NOT NULL DEFAULT 0,
    cost_fees         REAL    NOT NULL DEFAULT 0,  -- tasas/impuestos/landing/nav
    net               REAL    NOT NULL DEFAULT 0,
    gsx_matched       INTEGER NOT NULL DEFAULT 0,  -- 1 si hubo recibos GSX reales
    created_at        TEXT
);

CREATE INDEX IF NOT EXISTS idx_flight_pnl_airline ON flight_pnl(airline_key);
