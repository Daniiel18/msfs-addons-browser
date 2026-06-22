-- (v6.1) Registro de mantenimiento por aeronave. Cada vez que el jugador hace
-- un servicio (cambiar neumáticos, refill aceite, reset EGT…) se guarda aquí:
-- resetea el desgaste de ESE componente (se cuenta el desgaste por los vuelos
-- POSTERIORES al servicio) y su coste se suma a la economía de la aerolínea.
-- Es config/historial, NO se borra en recompute.

CREATE TABLE IF NOT EXISTS maintenance_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    registration TEXT    NOT NULL,   -- matrícula normalizada (alfanum, mayúsculas)
    component    TEXT    NOT NULL,   -- tires/brakes/engine_oil/hydraulics/...
    cost         REAL    NOT NULL DEFAULT 0,
    serviced_at  TEXT    NOT NULL    -- ISO-8601 UTC
);

CREATE INDEX IF NOT EXISTS idx_maintenance_log_reg
    ON maintenance_log(registration, component);
