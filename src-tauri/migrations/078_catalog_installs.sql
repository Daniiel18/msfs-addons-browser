-- (02/11) Qué versión del catálogo instaló SimFleet en cada carpeta de
-- Community. Un hotfix que no cambia el número del paquete ("1.2.8" →
-- "1.2.8 Hotfix" en aerohub) deja de salir como actualización pendiente en
-- cuanto se instala desde la app; vuelve a salir sólo si el catálogo trae
-- otra versión.
CREATE TABLE IF NOT EXISTS catalog_installs (
    folder_name  TEXT NOT NULL,
    addon_id     TEXT NOT NULL,
    source       TEXT,
    version      TEXT NOT NULL,
    installed_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (folder_name, addon_id)
);

-- Cuándo cambió por última vez la versión de cada ficha del catálogo (NULL =
-- desde que se vio por primera vez). Una descarga anterior a ese cambio no
-- cuenta como instalación de la versión nueva.
ALTER TABLE addons ADD COLUMN version_seen_at TEXT;
