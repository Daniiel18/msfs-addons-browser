-- (04/11) Etapa 9 — extras de la biblioteca.
--
-- addon_rating: estrellas del usuario, sólo en este PC. Va aparte de
-- community_packages para sobrevivir si el addon se desinstala y vuelve.
CREATE TABLE IF NOT EXISTS addon_rating (
    folder_name TEXT PRIMARY KEY COLLATE NOCASE,
    stars       INTEGER NOT NULL CHECK (stars BETWEEN 1 AND 5),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Última actualización real del addon (encender y apagar no la mueven, a
-- diferencia de folder_modified_at). La rellena el próximo escaneo.
ALTER TABLE community_packages ADD COLUMN last_update TEXT;
