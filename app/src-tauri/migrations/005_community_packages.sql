-- Inventario de paquetes en la carpeta Community de MSFS.
--
-- A diferencia de `installed_addons` (que sólo registra lo que
-- instalamos *a través de esta app*), esta tabla refleja todo lo
-- que existe físicamente en la carpeta Community — escaneada
-- periódicamente leyendo cada `manifest.json`.
--
-- Permite:
--   · Listar el catálogo real del usuario en el mapa, aunque haya
--     instalado paquetes manualmente o con otra herramienta.
--   · Detectar updates: comparar `package_version` con la versión
--     que el catálogo (`addons`) reporta para el mismo ICAO.
--   · Mostrar metadatos del manifest (creador, content_type,
--     compatibilidad de versión MSFS) sin tocar el disco cada vez.
--
-- La clave primaria es el nombre de la carpeta porque MSFS usa
-- ese nombre como identidad lógica del paquete (varias versiones
-- del "mismo" paquete coexisten sólo si tienen folders distintos).
CREATE TABLE IF NOT EXISTS community_packages (
    folder_name             TEXT PRIMARY KEY,
    install_path            TEXT NOT NULL,
    title                   TEXT NOT NULL,
    creator                 TEXT,
    content_type            TEXT,            -- SCENERY | AIRCRAFT | MISC | …
    package_version         TEXT,            -- "1.5.5" del manifest
    minimum_game_version    TEXT,
    icao                    TEXT,            -- normalizado a mayúsculas
    size_bytes              INTEGER,
    folder_modified_at      TEXT,
    scanned_at              TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_community_packages_icao ON community_packages(icao);
CREATE INDEX IF NOT EXISTS idx_community_packages_content_type ON community_packages(content_type);
CREATE INDEX IF NOT EXISTS idx_community_packages_creator ON community_packages(creator);
