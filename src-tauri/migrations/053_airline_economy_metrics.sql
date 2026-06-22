-- (v6.1) Métricas adicionales para la pantalla de Finanzas.
--
-- Va en una migración aparte (no editando la 052) porque la 052 ya pudo
-- aplicarse en una build anterior: sqlx la marca como aplicada por su número
-- y NO re-ejecuta el CREATE TABLE aunque cambie. Estas columnas se añaden con
-- ALTER sobre la tabla existente para que tanto las DB ya migradas como las
-- nuevas acaben con el mismo esquema.
--
-- Agregados materializados por aerolínea (los rellena `recompute`):
--   · passengers      — pasajeros totales transportados
--   · revenue_*       — desglose de ingresos (billetes / extras)
--   · cost_*          — desglose de costes (fuel/catering/handling/mant/fees)
--   · avg_landing_fpm — media de FPM al tocar pista (NULL si ningún vuelo lo midió)
--   · satisfaction    — 0-100, derivada del FPM (la "satisfacción del pasajero")
--   · gsx_flights     — nº de vuelos cuyos costes salieron de facturas GSX reales

ALTER TABLE airline_economy ADD COLUMN passengers        INTEGER NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN revenue_tickets   REAL    NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN revenue_ancillary REAL    NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN cost_fuel         REAL    NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN cost_catering     REAL    NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN cost_handling     REAL    NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN cost_maintenance  REAL    NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN cost_fees         REAL    NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN avg_landing_fpm   REAL;
ALTER TABLE airline_economy ADD COLUMN satisfaction      REAL;
ALTER TABLE airline_economy ADD COLUMN gsx_flights       INTEGER NOT NULL DEFAULT 0;
