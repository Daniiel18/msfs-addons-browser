/**
 * (v3.28.0 P7.11 · v3.39.0 #3) Sistema de unidades — POR CATEGORÍA.
 *
 * Toda la telemetría se ALMACENA en unidades canónicas (las que entrega
 * SimConnect / la DB): distancia=nm, altitud=ft, velocidad=kt,
 * vert.speed=fpm, peso=kg, temperatura=°C, presión=hPa, flujo=pph.
 *
 * Este módulo convierte ese valor canónico a la unidad elegida por el
 * usuario SÓLO al mostrarlo. Nunca se re-escribe la DB — es puramente de
 * presentación, así que cambiar una unidad es instantáneo y reversible.
 *
 * (v3.39.0 #3) Cada magnitud se elige por separado (peso, altitud,
 * velocidad, V/S, distancia, presión, temperatura). `useUnits()` lee las
 * 7 preferencias de `useSettingsStore` → cualquier componente que lo use
 * se re-renderiza al cambiarlas, sin reinicio. El flujo de combustible
 * sigue a la unidad de peso (kg→kg/h, lb→pph).
 */

import { useMemo } from "react";
import { useSettingsStore } from "../stores/useSettingsStore";
import type { UnitPrefs, UnitSystem, TempUnit } from "./unitsStorage";

export type {
  UnitSystem,
  TempUnit,
  UnitPrefs,
  WeightUnit,
  AltitudeUnit,
  SpeedUnit,
  VsUnit,
  DistanceUnit,
  PressureUnit,
} from "./unitsStorage";

// Factores de conversión desde la unidad canónica.
const NM_TO_KM = 1.852;
const NM_TO_MI = 1.150779; // nm → millas terrestres
const FT_TO_M = 0.3048;
const KT_TO_KMH = 1.852;
const KT_TO_MPH = 1.150779;
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
  /** (compat) "metric"|"imperial" derivado de la altitud, para los
   *  pocos sitios que aún razonan por "sistema" (p.ej. visibilidad). */
  system: UnitSystem;
  temp: TempUnit;

  /** Etiquetas de unidad activas (para ejes de gráficos). */
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

  /** Convierte el valor canónico → número en la unidad activa. */
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

  /** Formateadores completos (número redondeado + sufijo). `null` → "—". */
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

/** Construye el set de conversores/formateadores para unas prefs dadas.
 *  Pura — sin React. */
export function makeUnits(p: UnitPrefs): Units {
  const conv = {
    altitude: (ft: number) => (p.altitude === "m" ? ft * FT_TO_M : ft),
    speed: (kt: number) =>
      p.speed === "kmh" ? kt * KT_TO_KMH : p.speed === "mph" ? kt * KT_TO_MPH : kt,
    distance: (nm: number) =>
      p.distance === "km" ? nm * NM_TO_KM : p.distance === "mi" ? nm * NM_TO_MI : nm,
    vs: (fpm: number) => (p.vs === "ms" ? fpm * FPM_TO_MS : fpm),
    weight: (kg: number) => (p.weight === "lb" ? kg * KG_TO_LB : kg),
    temp: (c: number) => (p.temp === "F" ? c * 1.8 + 32 : c),
    pressure: (hpa: number) => (p.pressure === "inHg" ? hpa * HPA_TO_INHG : hpa),
    fuelFlow: (pph: number) => (p.weight === "kg" ? pph * PPH_TO_KGH : pph),
  };

  const unit = {
    altitude: p.altitude === "m" ? "m" : "ft",
    speed: p.speed === "kmh" ? "km/h" : p.speed === "mph" ? "mph" : "kt",
    distance: p.distance === "km" ? "km" : p.distance === "mi" ? "mi" : "nm",
    vs: p.vs === "ms" ? "m/s" : "fpm",
    weight: p.weight === "kg" ? "kg" : "lb",
    temp: p.temp === "F" ? "°F" : "°C",
    pressure: p.pressure === "inHg" ? "inHg" : "hPa",
    fuelFlow: p.weight === "kg" ? "kg/h" : "pph",
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
        : p.vs === "ms"
          ? `${conv.vs(fpm).toFixed(1)} ${unit.vs}`
          : `${Math.round(fpm)} ${unit.vs}`,
    weight: (kg: number | null | undefined) =>
      kg == null ? DASH : `${group(Math.round(conv.weight(kg)))} ${unit.weight}`,
    temp: (c: number | null | undefined) =>
      c == null ? DASH : `${Math.round(conv.temp(c))}${unit.temp}`,
    pressure: (hpa: number | null | undefined) =>
      hpa == null
        ? DASH
        : p.pressure === "inHg"
          ? `${conv.pressure(hpa).toFixed(2)} ${unit.pressure}`
          : `${Math.round(conv.pressure(hpa))} ${unit.pressure}`,
  };

  // (compat) "sistema" derivado: la visibilidad del Weather y similares
  // razonan por métrico/imperial; usamos la altitud como representante.
  const system: UnitSystem = p.altitude === "m" ? "metric" : "imperial";

  return { system, temp: p.temp, unit, conv, fmt };
}

// ────────────────────────────────────────────────────────────────────
// Hook reactivo
// ────────────────────────────────────────────────────────────────────

/** Devuelve los formateadores ligados a las unidades ACTUALES.
 *  Re-renderiza el componente cuando el usuario cambia cualquier unidad. */
export function useUnits(): Units {
  const weight = useSettingsStore((s) => s.settings.unitWeight);
  const altitude = useSettingsStore((s) => s.settings.unitAltitude);
  const speed = useSettingsStore((s) => s.settings.unitSpeed);
  const vs = useSettingsStore((s) => s.settings.unitVs);
  const distance = useSettingsStore((s) => s.settings.unitDistance);
  const pressure = useSettingsStore((s) => s.settings.unitPressure);
  const temp = useSettingsStore((s) => s.settings.tempUnit);
  return useMemo(
    () =>
      makeUnits({
        weight: weight === "kg" ? "kg" : "lb",
        altitude: altitude === "m" ? "m" : "ft",
        speed: speed === "kmh" ? "kmh" : speed === "mph" ? "mph" : "kt",
        vs: vs === "ms" ? "ms" : "fpm",
        distance: distance === "km" ? "km" : distance === "mi" ? "mi" : "nm",
        pressure: pressure === "hPa" ? "hPa" : "inHg",
        temp: temp === "F" ? "F" : "C",
      }),
    [weight, altitude, speed, vs, distance, pressure, temp],
  );
}
