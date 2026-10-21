-- (19/10) Sincronía con Fenix TechLog / PMDG TechLog.
-- Eventos de mantenimiento de TechLog ya vistos por SimFleet (servicios,
-- reparaciones/reemplazos de piezas, defectos rectificados, checks hechos),
-- para cobrarlos UNA sola vez en Finanzas (vía `maintenance_log`).
-- La primera vez que SimFleet ve un avión de TechLog registra su historial
-- como "visto" sin cobrarlo (fila `baseline:<fuente>:<matrícula>`).
CREATE TABLE IF NOT EXISTS techlog_seen (
    event_key    TEXT PRIMARY KEY,
    source       TEXT NOT NULL,
    registration TEXT NOT NULL,
    cost         REAL NOT NULL DEFAULT 0,
    seen_at      TEXT NOT NULL
);
