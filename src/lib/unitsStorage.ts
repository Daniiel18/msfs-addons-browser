/**
 * (v3.28.0 P7.11 · v3.39.0 #3) Espejo de las preferencias de unidades en
 * localStorage, lectura SÍNCRONA pre-render (igual que i18n).
 *
 * (v3.39.0 #3) Unidades POR CATEGORÍA: cada magnitud (peso, altitud,
 * velocidad, V/S, distancia, presión, temperatura) se elige por
 * separado. Los presets Imperial/Métrico son sólo un atajo de UI que
 * rellenan las 6 primeras de golpe (la temperatura es independiente).
 *
 * Vive en su propio módulo —SIN dependencias— para romper el ciclo
 * `units.ts` ⇄ `useSettingsStore.ts` (el store siembra su estado inicial
 * desde estos helpers, y `units.ts` lee el store).
 */

export type WeightUnit = "kg" | "lb";
export type AltitudeUnit = "ft" | "m";
export type SpeedUnit = "kt" | "kmh" | "mph";
export type VsUnit = "fpm" | "ms";
export type DistanceUnit = "nm" | "km" | "mi";
export type PressureUnit = "inHg" | "hPa";
export type TempUnit = "C" | "F";

/** (compat) Algunos sitios todavía hablan de "sistema" imperial/métrico
 *  (p.ej. la visibilidad del Weather). Se deriva de las prefs. */
export type UnitSystem = "imperial" | "metric";

export interface UnitPrefs {
  weight: WeightUnit;
  altitude: AltitudeUnit;
  speed: SpeedUnit;
  vs: VsUnit;
  distance: DistanceUnit;
  pressure: PressureUnit;
  temp: TempUnit;
}

/** Presets (sin temperatura — esa es independiente). */
export const IMPERIAL_PREFS: Omit<UnitPrefs, "temp"> = {
  weight: "lb",
  altitude: "ft",
  speed: "kt",
  vs: "fpm",
  distance: "nm",
  pressure: "inHg",
};
export const METRIC_PREFS: Omit<UnitPrefs, "temp"> = {
  weight: "kg",
  altitude: "m",
  speed: "kmh",
  vs: "ms",
  distance: "km",
  pressure: "hPa",
};

const LS = {
  weight: "simfleet.unit.weight",
  altitude: "simfleet.unit.altitude",
  speed: "simfleet.unit.speed",
  vs: "simfleet.unit.vs",
  distance: "simfleet.unit.distance",
  pressure: "simfleet.unit.pressure",
  temp: "simfleet.tempUnit", // (compat) reusa la clave de v3.28.0
} as const;

function read(key: string): string | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage.getItem(key) : null;
  } catch {
    return null;
  }
}

function pick<T extends string>(raw: string | null, allowed: readonly T[], fallback: T): T {
  return (allowed as readonly string[]).includes(raw ?? "") ? (raw as T) : fallback;
}

export function loadStoredUnitPrefs(): UnitPrefs {
  return {
    weight: pick(read(LS.weight), ["kg", "lb"] as const, "lb"),
    altitude: pick(read(LS.altitude), ["ft", "m"] as const, "ft"),
    speed: pick(read(LS.speed), ["kt", "kmh", "mph"] as const, "kt"),
    vs: pick(read(LS.vs), ["fpm", "ms"] as const, "fpm"),
    distance: pick(read(LS.distance), ["nm", "km", "mi"] as const, "nm"),
    pressure: pick(read(LS.pressure), ["inHg", "hPa"] as const, "inHg"),
    temp: pick(read(LS.temp), ["C", "F"] as const, "C"),
  };
}

export function persistUnitPrefs(p: UnitPrefs): void {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(LS.weight, p.weight);
    localStorage.setItem(LS.altitude, p.altitude);
    localStorage.setItem(LS.speed, p.speed);
    localStorage.setItem(LS.vs, p.vs);
    localStorage.setItem(LS.distance, p.distance);
    localStorage.setItem(LS.pressure, p.pressure);
    localStorage.setItem(LS.temp, p.temp);
  } catch {
    /* la DB sigue siendo la fuente de verdad */
  }
}
