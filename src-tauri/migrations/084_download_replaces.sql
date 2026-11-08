-- (04/11) Qué carpeta de Community sustituye una descarga (Reinstalar y
-- Actualizar). Sigue instalada hasta que lo nuevo termina de instalarse; se
-- guarda para que una descarga reanudada tras cerrar SimFleet no la olvide.
ALTER TABLE download_jobs ADD COLUMN replaces TEXT;
