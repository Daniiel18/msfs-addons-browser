-- Migración 002: relajar `installed_addons` para soportar instalaciones
-- manuales (archivos .zip/.rar/.7z que el usuario selecciona a mano).
--
-- Cambios:
--   1. `addon_id` pasa a ser NULLABLE — una instalación manual no tiene
--      un addon "origen" en la tabla `addons`.
--   2. FK cambia de ON DELETE CASCADE → ON DELETE SET NULL. Semántica
--      más natural para historial: si se limpia el caché de addons, la
--      instalación de disco sigue existiendo.
--   3. Se añaden columnas denormalizadas (`source`, `title`, `name`,
--      `developer`) para poder mostrar la fila sin hacer JOIN — crítico
--      para instalaciones manuales donde no hay `addon_id` que unir.
--
-- SQLite no soporta ALTER COLUMN, así que seguimos el patrón canónico:
-- renombrar la tabla vieja, crear la nueva con el schema deseado, copiar
-- los datos, y borrar la vieja.

ALTER TABLE installed_addons RENAME TO installed_addons_v1;

CREATE TABLE installed_addons (
    id              TEXT PRIMARY KEY,
    addon_id        TEXT REFERENCES addons(id) ON DELETE SET NULL,
    -- Denormalizados para display (sobre todo en instalaciones manuales).
    source          TEXT,
    title           TEXT NOT NULL,
    name            TEXT,
    developer       TEXT,
    version         TEXT,
    install_path    TEXT NOT NULL,
    size_bytes      INTEGER,
    installed_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Poblado con lo que había más los datos que podamos derivar de `addons`.
-- Para filas históricas: si existe el addon, copiamos sus campos; si no,
-- usamos el `install_path` como título (es feo pero evita dejar el
-- campo en NULL, y el usuario puede reinstalar para limpiarlo).
INSERT INTO installed_addons (
    id, addon_id, source, title, name, developer, version,
    install_path, size_bytes, installed_at
)
SELECT
    v1.id,
    v1.addon_id,
    (SELECT a.source    FROM addons a WHERE a.id = v1.addon_id),
    COALESCE(
        (SELECT a.name FROM addons a WHERE a.id = v1.addon_id),
        v1.install_path
    ),
    (SELECT a.name      FROM addons a WHERE a.id = v1.addon_id),
    (SELECT a.developer FROM addons a WHERE a.id = v1.addon_id),
    v1.version,
    v1.install_path,
    v1.size_bytes,
    v1.installed_at
FROM installed_addons_v1 AS v1;

DROP TABLE installed_addons_v1;

CREATE INDEX IF NOT EXISTS idx_installed_addon_id ON installed_addons(addon_id);
CREATE INDEX IF NOT EXISTS idx_installed_installed_at ON installed_addons(installed_at);
