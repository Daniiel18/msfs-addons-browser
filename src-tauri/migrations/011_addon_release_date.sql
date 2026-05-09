-- Fecha de publicación del addon en su fuente original.
--
-- Tanto SceneryAddons como Simplaza son blogs WordPress y exponen
-- la fecha de cada post en `<time datetime="...">` o `<meta
-- property="article:published_time">` en la página de detalle. La
-- guardamos para que el catálogo de la app muestre "subido el X" —
-- el usuario reportó que sólo aparecía título/dev/versión.
--
-- TEXT en formato ISO-8601 ("2024-09-15T10:23:00+02:00"). NULL
-- cuando el parser no la pudo extraer (posts viejos sin metadata).
ALTER TABLE addons ADD COLUMN released_at TEXT;
CREATE INDEX IF NOT EXISTS idx_addons_released_at ON addons(released_at);
