-- (v6.1) Ecosistema de gestión de aerolínea: decisiones del jugador que
-- alimentan el P&L. Una fila por aerolínea (clave canónica = `airline_key`).
--
--   · maintenance_level — 0 básico / 1 estándar / 2 premium. Sube el coste de
--     mantenimiento pero mejora la satisfacción/fiabilidad.
--   · servicios a bordo (snacks/meals/wifi/asientos/embarque/equipaje) — cada
--     uno activa un ingreso ancillary por pasajero (y algo de satisfacción).
--
-- Es configuración, no se borra en cada recompute (a diferencia de
-- airline_economy / flight_pnl, que son caché). `recompute` la LEE.

CREATE TABLE IF NOT EXISTS airline_policy (
    airline_key       TEXT PRIMARY KEY,
    maintenance_level INTEGER NOT NULL DEFAULT 1,
    snacks            INTEGER NOT NULL DEFAULT 0,
    meals             INTEGER NOT NULL DEFAULT 0,
    wifi              INTEGER NOT NULL DEFAULT 0,
    seat_upgrades     INTEGER NOT NULL DEFAULT 0,
    priority_boarding INTEGER NOT NULL DEFAULT 0,
    extra_baggage     INTEGER NOT NULL DEFAULT 0,
    updated_at        TEXT
);
