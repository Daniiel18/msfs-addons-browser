-- (24/10) Lo que el usuario borró a propósito no vuelve de la nube.
--
-- `kind`: flight (clave estable del vuelo: "startedAt|lat|lon" o
-- "source|externalId") | maint (cobro de mantenimiento quitado). La nube
-- guarda la unión de las de todas las PCs: borrar en una borra en las demás.
-- `revived_at`: el vuelo se volvió a añadir después (p.ej. reimportado); el
-- borrado vale mientras `deleted_at` sea más reciente.
CREATE TABLE IF NOT EXISTS sync_tombstone (
    kind        TEXT NOT NULL,
    key         TEXT NOT NULL,
    deleted_at  TEXT NOT NULL,
    revived_at  TEXT,
    PRIMARY KEY (kind, key)
);

-- Versión de cada avión del Tech Log de SimFleet en la nube: si su contenido
-- cambia en esta PC, su `updated_at` pasa a ahora; entre PCs gana la versión
-- más reciente del avión entero.
CREATE TABLE IF NOT EXISTS sync_airframe_rev (
    registration  TEXT PRIMARY KEY,
    hash          TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
