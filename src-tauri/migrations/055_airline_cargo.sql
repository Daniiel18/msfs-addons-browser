-- (v6.1) Marca de aerolínea de CARGA en el ledger (modelo económico distinto:
-- ingreso por flete, sin pasajeros ni servicios a bordo). Columna nueva sobre
-- la tabla existente para no re-tocar migraciones ya aplicadas.
ALTER TABLE airline_economy ADD COLUMN is_cargo INTEGER NOT NULL DEFAULT 0;
