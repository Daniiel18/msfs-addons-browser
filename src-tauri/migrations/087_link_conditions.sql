-- (05/11) Condición de un enlace del Link Map: el enlace solo vale en ciertos
-- vuelos. JSON con las listas `country`, `airport`, `aircraft` y `airline`.
-- NULL = sin condición, como hasta ahora. Solo en enlaces manuales.
ALTER TABLE addon_links ADD COLUMN conditions_json TEXT;
