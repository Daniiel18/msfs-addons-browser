-- 063 (v7.2.2) Señal DEFINITIVA de avión: ¿el paquete trae su MODELO DE VUELO
-- (flight_model.cfg / .air) en algún container SimObjects/Airplanes? Reemplaza a
-- has_own_model para clasificar Aircraft — vale para los aviones ENCRIPTADOS
-- (PMDG/Fenix/iniBuilds, con has_own_model=false) y excluye liveries y mods
-- (cabina/luces/texturas) que declaran content_type=AIRCRAFT sin serlo.
-- Default 0; el siguiente escaneo de Community lo recalcula para todas las filas.
ALTER TABLE community_packages ADD COLUMN has_flight_model INTEGER NOT NULL DEFAULT 0;
