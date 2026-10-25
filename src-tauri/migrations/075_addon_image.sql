-- (25/10) Imagen del catálogo de cada addon: el modal de updates pendientes
-- la enseña como las tarjetas de Explorar.
ALTER TABLE addons ADD COLUMN image_url TEXT;

-- Que el listado de aerohub se relea en el próximo arranque para rellenarla
-- (si no, esperaría hasta 12 h).
DELETE FROM settings WHERE key = 'aerohub_listing_scan_at';
