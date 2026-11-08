-- (02/11) Etapa 5b — Cockpit del Tech Log propio: perfiles de interruptores
-- estándar del simulador por avión (matrícula normalizada) y su estado.
CREATE TABLE IF NOT EXISTS cockpit_std_profile (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    registration  TEXT NOT NULL,
    name          TEXT NOT NULL,
    -- Se pone solo al cargar el avión en frío y oscuro.
    auto          INTEGER NOT NULL DEFAULT 0,
    -- JSON: id del interruptor -> valor guardado.
    switches      TEXT NOT NULL DEFAULT '{}',
    saved_at      TEXT NOT NULL,
    saved_airport TEXT
);
CREATE INDEX IF NOT EXISTS idx_cockpit_std_profile_reg ON cockpit_std_profile (registration);

CREATE TABLE IF NOT EXISTS cockpit_std_state (
    registration    TEXT PRIMARY KEY,
    profile_id      INTEGER,
    -- «Restaurar al cargar en frío y oscuro» (el perfil elegido).
    restore_on_load INTEGER NOT NULL DEFAULT 0,
    -- JSON: ids de interruptores que NO entran en los perfiles.
    skip            TEXT NOT NULL DEFAULT '[]'
);
