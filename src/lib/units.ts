/**
 * (v3.28.0 — P7.11) Sistema de unidades global.
 *
 * Toda la telemetría se ALMACENA en unidades canónicas (las que
 * entrega SimConnect / la DB):
 *
 *   · distancia  → millas náuticas (nm)
 *   · altitud    → pies (ft)
 *   · velocidad  → nudos (kt)
 *   · vert.speed → pies/min (fpm)
 *   · peso       → kilogramos (kg)   ← la app guarda cargo/fuel en kg
 *   · temperatura→ grados Celsius (°C)
 *   · presión    → hectopascales (hPa)
 *   · flujo comb.→ libras/hora (pph)
 *
 * Este módulo convierte ESE valor canónico al sistema elegido por el
 * usuario SÓLO al mostrarlo. Nunca se re-escribe la DB — el toggle es
 * puramente de presentación, así que cambiarlo es instantáneo y
 * reversible.
 *
 * ## Reactividad
 *
 * `useUnits()` lee `unitSystem` + `tempUnit` de `useSettingsStore`
 * (zustand) → cualquier componente que lo use se re-renderiza al
 * cambiar el toggle, sin reinicio (a diferencia del idioma, que sí
 * requiere restart porque `t()` no es reactivo).
 *
 * ## Espejo en localStorage
 *
 * Igual que i18n: `preloadUnits()` lee el valor síncronamente antes
 * del primer render para que el cold-start ya pinte en el sistema
 * correcto, y la store espeja a localStorage al guardar / al
 * bootstrap (cubre el caso "cloud sync trajo prefs de otra PC").
 */

import { useMemo } from "react";
import { useSettingsStore } from "../stores/useSettingsStore";
import type { UnitSystem, TempUnit } from "./unitsStorage";

export type { UnitSystem, TempUnit } from "./unitsStorage";

// Factores de conversión desde la unidad canónica.
const NM_TO_KM = 1.852;
const FT_TO_M = 0.3048;
const KT_TO_KMH = 1.852;
const FPM_TO_MS = 0.00508; // 1 ft/min = 0.00508 m/s
const KG_TO_LB = 2.2046226218;
const HPA_TO_INHG = 0.0295299830714;
const PPH_TO_KGH = 0.45359237;

/** Agrupación de miles consistente con el resto de la app ("41,000"). */
function group(n: number): string {
  return n.toLocaleString("en-US");
}

const DASH = "—";

export interface Units {
  system: UnitSystem;
  temp: TempUnit;

  /** Etiquetas de unidad del sistema activo (para ejes de gráficos). */
  unit: {
    altitude: string;
    speed: string;
    distance: string;
    vs: string;
    weight: string;
    temp: string;
    pressure: string;
    fuelFlow: string;
  };

  /** Convierte el valor canónico → número en la unidad activa (sin
   *  formatear). Útil para ejes/series de Recharts. */
  conv: {
    altitude: (ft: number) => number;
    speed: (kt: number) => number;
    distance: (nm: number) => number;
    vs: (fpm: number) => number;
    weight: (kg: number) => number;
    temp: (c: number) => number;
    pressure: (hpa: number) => number;
    fuelFlow: (pph: number) => number;
  };

  /** Formateadores completos: número (redondeado + agrupado) + sufijo.
   *  `null`/`undefined` → "—". */
  fmt: {
    altitude: (ft: number | null | undefined) => string;
    speed: (kt: number | null | undefined) => string;
    distance: (nm: number | null | undefined) => string;
    vs: (fpm: number | null | undefined) => string;
    weight: (kg: number | null | undefined) => string;
    temp: (c: number | null | undefined) => string;
    pressure: (hpa: number | null | undefined) => string;
  };
}

/** Construye el set de conversores/formateadores para un sistema dado.
 *  Pura — sin React. Exportada por si se necesita fuera de un hook. */
export function makeUnits(system: UnitSystem, temp: TempUnit): Units {
  const metric = system === "metric";
  const fahrenheit = temp === "F";

  const conv = {
    altitude: (ft: number) => (metric ? ft * FT_TO_M : ft),
    speed: (kt: number) => (metric ? kt * KT_TO_KMH : kt),
    distance: (nm: number) => (metric ? nm * NM_TO_KM : nm),
    vs: (fpm: number) => (metric ? fpm * FPM_TO_MS : fpm),
    weight: (kg: number) => (metric ? kg : kg * KG_TO_LB),
    temp: (c: number) => (fahrenheit ? c * 1.8 + 32 : c),
    pressure: (hpa: number) => (metric ? hpa : hpa * HPA_TO_INHG),
    fuelFlow: (pph: number) => (metric ? pph * PPH_TO_KGH : pph),
  };

  const unit = {
    altitude: metric ? "m" : "ft",
    speed: metric ? "km/h" : "kt",
    distance: metric ? "km" : "nm",
    vs: metric ? "m/s" : "fpm",
    weight: metric ? "kg" : "lb",
    temp: fahrenheit ? "°F" : "°C",
    pressure: metric ? "hPa" : "inHg",
    fuelFlow: metric ? "kg/h" : "pph",
  };

  const fmt = {
    altitude: (ft: number | null | undefined) =>
      ft == null ? DASH : `${group(Math.round(conv.altitude(ft)))} ${unit.altitude}`,
    speed: (kt: number | null | undefined) =>
      kt == null ? DASH : `${group(Math.round(conv.speed(kt)))} ${unit.speed}`,
    distance: (nm: number | null | undefined) =>
      nm == null ? DASH : `${group(Math.round(conv.distance(nm)))} ${unit.distance}`,
    vs: (fpm: number | null | undefined) =>
      fpm == null
        ? DASH
        : metric
          ? `${conv.vs(fpm).toFixed(1)} ${unit.vs}`
          : `${Math.round(fpm)} ${unit.vs}`,
    weight: (kg: number | null | undefined) =>
      kg == null ? DASH : `${group(Math.round(conv.weight(kg)))} ${unit.weight}`,
    temp: (c: number | null | undefined) =>
      c == null ? DASH : `${Math.round(conv.temp(c))}${unit.temp}`,
    pressure: (hpa: number | null | undefined) =>
      hpa == null
        ? DASH
        : metric
          ? `${Math.round(conv.pressure(hpa))} ${unit.pressure}`
          : `${conv.pressure(hpa).toFixed(2)} ${unit.pressure}`,
  };

  return { system, temp, unit, conv, fmt };
}

// ────────────────────────────────────────────────────────────────────
// Hook reactivo
// ────────────────────────────────────────────────────────────────────

/** Devuelve los formateadores ligados al sistema de unidades ACTUAL.
 *  Re-renderiza el componente cuando el usuario cambia el toggle. */
export function useUnits(): Units {
  const system = useSettingsStore((s) => s.settings.unitSystem) as UnitSystem;
  const temp = useSettingsStore((s) => s.settings.tempUnit) as TempUnit;
  return useMemo(
    () => makeUnits(system === "metric" ? "metric" : "imperial", temp === "F" ? "F" : "C"),
    [system, temp],
  );
}
