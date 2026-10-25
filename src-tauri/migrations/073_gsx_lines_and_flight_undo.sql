-- (24/10) Contabilidad sólida.
--
-- Cada línea de cada recibo de GSX, una sola vez (receipt_id + line) y como
-- mucho en UN vuelo: el desembarque va al vuelo que llega, el embarque,
-- pushback, catering y desabastecimiento al que sale. Las cargas de combustible
-- no son gasto (is_cost = 0): dan el precio por kg del aeropuerto. Es caché
-- de `recompute` (se reconstruye entera desde los recibos y los vuelos).
CREATE TABLE IF NOT EXISTS gsx_charge (
    receipt_id    TEXT NOT NULL,
    line          INTEGER NOT NULL,
    kind          TEXT NOT NULL,              -- catering | handling | bus | fuel
    service       TEXT NOT NULL,
    side          TEXT NOT NULL,              -- dep | arr
    airport       TEXT NOT NULL,
    registration  TEXT NOT NULL,
    at            TEXT NOT NULL,              -- hora REAL (archivo), no la del sim
    amount_usd    REAL NOT NULL,              -- con su parte de impuestos
    uplift_kg     REAL,                       -- sólo cargas de combustible
    is_cost       INTEGER NOT NULL,
    flight_id     INTEGER,                    -- NULL = sin vuelo (no se cobra)
    PRIMARY KEY (receipt_id, line)
);
CREATE INDEX IF NOT EXISTS idx_gsx_charge_flight ON gsx_charge (flight_id);
CREATE INDEX IF NOT EXISTS idx_gsx_charge_airport ON gsx_charge (airport);

-- Combustible del vuelo: quemado × precio por kg (GSX de ese aeropuerto o
-- referencia), para poder explicar el costo.
ALTER TABLE flight_pnl ADD COLUMN fuel_kg REAL;
ALTER TABLE flight_pnl ADD COLUMN fuel_price_kg REAL;

-- Lo que un vuelo cambió en el Tech Log de SimFleet (antes/después de
-- contadores, consumibles, margen EGT y piezas): borrar el vuelo lo revierte.
ALTER TABLE mx_flight ADD COLUMN undo TEXT;
