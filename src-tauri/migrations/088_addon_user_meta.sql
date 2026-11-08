-- (06/11) Tipo y etiquetas que el usuario da a un addon. Solo en este PC y nunca
-- en los archivos del addon. Va aparte de community_packages para sobrevivir si
-- el addon se desinstala y vuelve, como las estrellas.
CREATE TABLE IF NOT EXISTS addon_user_meta (
    folder_name TEXT PRIMARY KEY COLLATE NOCASE,
    user_type   TEXT,
    tags        TEXT NOT NULL DEFAULT '[]',
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
