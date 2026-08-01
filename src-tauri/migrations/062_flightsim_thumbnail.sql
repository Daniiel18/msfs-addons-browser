-- 062 (v6.2.75) Guarda la URL de la imagen de flightsim.to del addon rastreado,
-- para mostrarla en Addons (card + modal de detalle) en vez del placeholder.
ALTER TABLE flightsim_tracked_files ADD COLUMN thumbnail TEXT;
