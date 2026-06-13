-- (v4.28.0) Datos del aircraft.cfg / SimObjects para distinguir
-- aviones reales (self-contained) de liveries (base_container apunta a
-- otro paquete) y mods (cockpit, sonido). Persistido como JSON para
-- evitar tablas hijas: cada paquete suele tener 1-3 contenedores y
-- 1-2 base_containers.

ALTER TABLE community_packages ADD COLUMN simobject_dirs_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE community_packages ADD COLUMN base_containers_json TEXT NOT NULL DEFAULT '[]';
