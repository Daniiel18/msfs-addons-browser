-- (05/11) Addons que en Community son un enlace, y los que están también en
-- Community2024. Los recalcula cada escaneo.
ALTER TABLE community_packages ADD COLUMN link_target TEXT;
ALTER TABLE community_packages ADD COLUMN link_managed INTEGER NOT NULL DEFAULT 0;
ALTER TABLE community_packages ADD COLUMN also_in TEXT;
