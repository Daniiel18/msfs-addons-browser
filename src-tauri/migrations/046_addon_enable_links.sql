-- (v4.25.0) Enable/Disable de addons + grafo de dependencias (Link Map).
--
-- · `enabled` en community_packages: espejo en DB del estado físico del
--   paquete. La fuente de verdad es el filesystem (layout.json presente
--   = activo; layout.json.disabled = desactivado) — el scanner
--   recalcula esta columna en cada scan.
-- · `addon_links`: aristas dirigidas del grafo source → target
--   (aircraft → livery/soundpack). `origin` distingue las sembradas
--   automáticamente desde manifest.dependencies ('auto') de las que el
--   usuario crea a mano en el Link Map ('manual') — los re-scans
--   refrescan las 'auto' sin tocar las 'manual'.
-- · `addon_link_positions`: posición (x, y) de cada nodo en el lienzo
--   del Link Map para que el layout que el usuario arrastró persista.

ALTER TABLE community_packages ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1;

CREATE TABLE IF NOT EXISTS addon_links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_folder TEXT NOT NULL,
    target_folder TEXT NOT NULL,
    origin TEXT NOT NULL DEFAULT 'manual',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(source_folder, target_folder)
);

CREATE INDEX IF NOT EXISTS idx_addon_links_source ON addon_links(source_folder);
CREATE INDEX IF NOT EXISTS idx_addon_links_target ON addon_links(target_folder);

CREATE TABLE IF NOT EXISTS addon_link_positions (
    folder_name TEXT PRIMARY KEY,
    x REAL NOT NULL,
    y REAL NOT NULL
);
