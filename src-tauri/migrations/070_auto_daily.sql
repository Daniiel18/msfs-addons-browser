-- (v8.3 · Etapa 6.2c) Operación autónoma de las aerolíneas de Finanzas.
--
-- Por cada día UTC con sus 12 ventanas de OpenSky completas, P&L de los
-- vuelos reales de cada aerolínea (mismo modelo que los vuelos del usuario)
-- más los costos fijos de su flota (vistos volando en 30 días + aviones del
-- usuario; 4.000 / 8.000 / 22.000 USD por avión regional / estrecho / ancho).
-- Los días se recalculan mientras existan sus vuelos crudos (35 días) y
-- después quedan congelados: el historial no se pierde.
CREATE TABLE IF NOT EXISTS auto_daily (
    airline_key       TEXT    NOT NULL,
    day               TEXT    NOT NULL,      -- AAAA-MM-DD (UTC)
    flights           INTEGER NOT NULL DEFAULT 0,
    pax               INTEGER NOT NULL DEFAULT 0,
    revenue_tickets   REAL    NOT NULL DEFAULT 0,
    revenue_ancillary REAL    NOT NULL DEFAULT 0,
    cost_fuel         REAL    NOT NULL DEFAULT 0,
    cost_catering     REAL    NOT NULL DEFAULT 0,
    cost_handling     REAL    NOT NULL DEFAULT 0,
    cost_maintenance  REAL    NOT NULL DEFAULT 0,
    cost_fees         REAL    NOT NULL DEFAULT 0,
    cost_fixed        REAL    NOT NULL DEFAULT 0,
    fleet             INTEGER NOT NULL DEFAULT 0,
    computed_at       TEXT    NOT NULL,
    PRIMARY KEY (airline_key, day)
);

-- Finanzas: totales de la operación autónoma por aerolínea (ya incluidos en
-- revenue / costs y en el desglose por categoría).
ALTER TABLE airline_economy ADD COLUMN auto_flights INTEGER NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN auto_days INTEGER NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN auto_revenue REAL NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN auto_costs REAL NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN cost_fixed REAL NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN auto_fleet INTEGER NOT NULL DEFAULT 0;
