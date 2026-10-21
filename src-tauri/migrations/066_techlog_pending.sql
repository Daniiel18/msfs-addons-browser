-- (20/10) Etapa 4 — órdenes de mantenimiento para Fenix/PMDG TechLog que no
-- se pudieron aplicar al momento: TechLog sólo deja elegir avión cuando el
-- simulador no tiene uno cargado. Se aplican solas cuando TechLog tiene ese
-- avión activo. `request` = la orden tal cual (JSON de TlActionRequest).
-- status: pending | applied | failed | cancelled.
CREATE TABLE IF NOT EXISTS techlog_pending (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    source       TEXT NOT NULL,
    registration TEXT NOT NULL,
    action       TEXT NOT NULL,
    label        TEXT,
    request      TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending',
    message      TEXT,
    done_at      TEXT
);

CREATE INDEX IF NOT EXISTS idx_techlog_pending_open
    ON techlog_pending (status, source, registration);
