-- (v4.22.0) Reset de scores cacheados tras alinear la Flight Evaluation
-- con las reglas de aviacion real (auditoria del log CTV210):
--   1. taxi_out_speed ya no cuenta el inicio del takeoff roll.
--   2. final_approach_spoilers_stowed eliminada (el SOP exige spoilers
--      ARMADOS en final) -> nueva landing_spoilers_deployed (ground
--      spoilers desplegados en el touchdown).
--   3. landing_smooth: pase hasta -600 fpm (umbral real de inspeccion
--      por hard landing); antes fallaba un toque firme de -517.
--   4. taxi_in_speed ignora la salida rapida de pista (rapid exit <= 50 kt).
--   5. taxi_in_taxi_light excluye el tramo final llegando al gate.
-- El recompute ocurre al abrir la Flight Evaluation - sin re-volar.
DELETE FROM flight_log_score_item;
UPDATE flight_log SET score_total = NULL, score_max = NULL, score_grade = NULL;
