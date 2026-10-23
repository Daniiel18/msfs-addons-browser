-- (v8.3) Finanzas: pasajeros y carga por aerolínea, y curva de saldo por hora.
--
-- * Cada vuelo se valora como carga o pasaje por sí mismo (carguero, aerolínea
--   de carga o carga con <= 3 personas a bordo); los cargueros sólo llevan
--   jumpseats. Se guarda la carga transportada (kg).
-- * La operación autónoma guarda pasajeros y carga estimados por día y el
--   neto por hora UTC (`hourly`, JSON [[vuelos, neto], ...] x 24) para que la
--   curva de saldo se mueva con los vuelos reales.
ALTER TABLE airline_economy ADD COLUMN cargo_kg REAL NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN auto_pax INTEGER NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN auto_cargo_kg REAL NOT NULL DEFAULT 0;
ALTER TABLE auto_daily ADD COLUMN cargo_kg REAL NOT NULL DEFAULT 0;
ALTER TABLE auto_daily ADD COLUMN hourly TEXT;
