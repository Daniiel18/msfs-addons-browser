/**
 * (v3.28.0 — P7.11) Espejo de las preferencias de unidades en
 * localStorage, lectura SÍNCRONA pre-render (igual que i18n).
 *
 * Vive en su propio módulo —SIN dependencias— para romper el ciclo
 * `units.ts` ⇄ `useSettingsStore.ts`:
 *   · `useSettingsStore` necesita estos helpers al construirse (estado
 *     inicial sembrado desde localStorage).
 *   · `units.ts` (el hook `useUnits`) necesita leer el store.
 * Si los helpers vivieran en `units.ts`, el `create()` del store podría
 * invocarlos mientras `units.ts` aún está a medio evaluar → los `const`
 * de las claves estarían en la TDZ → ReferenceError. Aislarlos aquí
 * elimina el ciclo por completo.
 */

export type UnitSystem = "imperial" | "metric";
export type TempUnit = "C" | "F";

const LS_SYSTEM = "simfleet.unitSystem";
const LS_TEMP = "simfleet.tempUnit";

export function loadStoredUnitSystem(): UnitSystem {
  try {
    const v =
      typeof localStorage !== "undefined" ? localStorage.getItem(LS_SYSTEM) : null;
    return v === "metric" ? "metric" : "imperial";
  } catch {
    return "imperial";
  }
}

export function loadStoredTempUnit(): TempUnit {
  try {
    const v =
      typeof localStorage !== "undefined" ? localStorage.getItem(LS_TEMP) : null;
    return v === "F" ? "F" : "C";
  } catch {
    return "C";
  }
}

export function persistUnits(system: string, temp: string): void {
  try {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(LS_SYSTEM, system);
      localStorage.setItem(LS_TEMP, temp);
    }
  } catch {
    /* la DB sigue siendo la fuente de verdad */
  }
}
