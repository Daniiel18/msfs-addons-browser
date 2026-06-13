-- (v4.31.0) Señales estructurales para distinguir un AVIÓN REAL de
-- una modificación (texturas de cabina, soundset, tool) que declaran
-- content_type=AIRCRAFT en su manifest pero no aportan un avión.
--
-- · `has_own_model`: 1 si algún container bajo SimObjects/Airplanes
--   tiene una carpeta `model*` con geometría 3D propia. Verificado
--   con la Community real del usuario: el PMDG 777-300ER de PMDG
--   Simulations tiene `model.300ER/`; las texturas de cabina de
--   zHUES sólo tienen `texture.vc/` (sin model) → no son aviones.
-- · `has_manifest`: 0 si el paquete NO trae manifest.json — por
--   lógica es contenido 3rd-party no estándar (badge en la UI).
ALTER TABLE community_packages ADD COLUMN has_own_model INTEGER NOT NULL DEFAULT 0;
ALTER TABLE community_packages ADD COLUMN has_manifest INTEGER NOT NULL DEFAULT 1;
