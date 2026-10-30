-- (25/10) Descargas que se pueden pausar y reanudar, también tras reiniciar:
-- cómo retomar una descarga directa de SimFleet (JSON: enlace, archivo
-- parcial y qué hacer al terminar) y la imagen del addon para el panel.
ALTER TABLE download_jobs ADD COLUMN resume TEXT;
ALTER TABLE download_jobs ADD COLUMN image_url TEXT;
