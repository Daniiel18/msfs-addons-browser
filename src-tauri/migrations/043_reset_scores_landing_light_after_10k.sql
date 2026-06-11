-- (v4.17.0) Reset de scores cacheados tras corregir la regla de luces de
-- aterrizaje: ahora se evalúa por ALTITUD INDICADA (primer cruce ascendente
-- de 11.000 ft = 10k + margen) y el label pasa de "before" a "after
-- 10,000 ft". El evaluador anterior miraba el último sample del initial
-- climb (~9.9k), donde llevar la luz encendida ES correcto → ✗ injusto.
-- El recompute ocurre al abrir la Flight Evaluation — sin re-volar.
DELETE FROM flight_log_score_item;
UPDATE flight_log SET score_total = NULL, score_max = NULL, score_grade = NULL;
