-- (v4.0.0 P7.8c) Fase de scoring POR SAMPLE.
--
-- Causa raíz del "No data to evaluate": el watcher escribía el track
-- sample y la fase (en flight_log_phase) en DOS spawns async con DOS
-- timestamps independientes. Para la primera muestra de cada fase, el
-- ts del track podía ser microsegundos MENOR que el entered_at de la
-- fase → el sample quedaba FUERA de la ventana [entered, exited]. En
-- fases largas (cruise) había samples posteriores que sí caían dentro;
-- en fases cortas (initial_climb, final_approach) TODOS quedaban al
-- borde → "No data to evaluate".
--
-- Solución (lo que hace el ACARS/VAS): asignar la fase a CADA sample
-- directamente. El evaluador filtra por esta columna (`scoring_phase`)
-- en vez de hacer un JOIN temporal frágil contra flight_log_phase.
--
-- flight_log_phase se conserva (compat + posible uso UI), pero el
-- scoring ya no depende de su ventana temporal.
ALTER TABLE flight_log_track ADD COLUMN scoring_phase TEXT;

CREATE INDEX IF NOT EXISTS idx_flight_log_track_scoring_phase
  ON flight_log_track(flight_id, scoring_phase);
