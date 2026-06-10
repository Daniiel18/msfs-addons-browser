-- (v4.15.0 #3/#4) Reset de scores cacheados tras la reingeniería de Flight
-- Evaluation para compatibilidad con aviones de alta fidelidad:
--   · Parking brake: umbrales tolerantes a la lectura intermitente (PMDG/iFly).
--   · Pushback "engines off at start": regla ELIMINADA (arrancar motores
--     durante el pushback es procedimiento normal, no una falta).
--   · Taxi light: solo penaliza con el avión en movimiento + margen ~30-45s.
--   · Initial climb flaps: limpio antes de 10.000 ft (mín de flaps en ascenso).
--   · Landing lights: margen regulatorio hasta 11.000 ft.
--   · Velocidad sub-10.000 ft: tolerancia técnica a 261 kt (ráfagas/turbulencia).
--
-- Los fixes viven en el evaluador (rubric.rs), que re-lee la telemetría ya
-- guardada por sample. Basta con limpiar el cache: al abrir la Flight
-- Evaluation, el scoring recomputa con la lógica corregida y elimina de paso
-- el item obsoleto de la regla de pushback — sin re-volar nada.
DELETE FROM flight_log_score_item;
UPDATE flight_log SET score_total = NULL, score_max = NULL, score_grade = NULL;
