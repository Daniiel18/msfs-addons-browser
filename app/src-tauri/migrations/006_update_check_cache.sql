-- Cache por (ICAO, source) del último chequeo de updates contra el
-- catálogo. Sin este cache cada apertura dispara N×M búsquedas
-- (ICAOs instalados × fuentes activas) que en una librería
-- mediana son varios minutos.
--
-- TTL recomendado: 6 horas. Lo bastante alto para no machacar las
-- fuentes en sesiones cortas, pero corto frente al ciclo típico
-- de updates de scenery (días-semanas).
--
-- `last_known_version` se persiste para poder detectar cambios sin
-- necesidad de volver a hacer JOIN con `addons` en cada render —
-- pero la fuente de verdad sigue siendo `addons` (esta tabla es
-- estrictamente cache).
CREATE TABLE IF NOT EXISTS update_check_cache (
    icao                TEXT NOT NULL,
    source              TEXT NOT NULL,
    last_known_version  TEXT,                       -- NULL = la fuente no devolvió match
    catalog_addon_id    TEXT,                       -- referencia al `addons.id` ganador
    checked_at          TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (icao, source)
);

CREATE INDEX IF NOT EXISTS idx_update_cache_checked_at ON update_check_cache(checked_at);
