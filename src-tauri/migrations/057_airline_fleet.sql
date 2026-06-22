-- (v6.1) Inversión en flota: cada avión distinto (matrícula/livery que vuela la
-- aerolínea) cuenta como una COMPRA de ese avión. Se acumula su precio de
-- adquisición (por tipo) y se descuenta del saldo de la aerolínea.
ALTER TABLE airline_economy ADD COLUMN fleet_size  INTEGER NOT NULL DEFAULT 0;
ALTER TABLE airline_economy ADD COLUMN fleet_value REAL    NOT NULL DEFAULT 0;
