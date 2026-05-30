/**
 * (v3.4.0 → v3.5.0) Sistema i18n centralizado.
 *
 * Arquitectura:
 *   · Diccionarios **en JSON files** — `i18n/es.json` y `i18n/en.json`.
 *     Importados con la magia de Vite/TS para `resolveJsonModule`,
 *     emitidos como object literal en el bundle.
 *   · `t(key, args?)` hace lookup en el dict del idioma activo; cae
 *     a EN; si tampoco existe, devuelve la clave cruda.
 *   · El idioma activo vive en `useSettingsStore.settings.language`
 *     (persistido en SQLite). **Default `"auto"`** → resuelve contra
 *     `navigator.language` (locale del SO en Tauri 2).
 *   · **(v3.4.0) `preloadLocale()` lee `localStorage["simfleet.locale"]`
 *     ANTES del primer render** — sin esto el shell se montaba en EN
 *     porque el bootstrap de DB es async y `setActiveLocale` corría
 *     después del primer paint. La store mirror al guardar / al
 *     bootstrappear vía `persistLocale()`.
 *   · Cambio manual de idioma persiste a DB + localStorage y dispara
 *     un modal "reinicia para aplicar" — no re-renderizamos en
 *     caliente porque los `t()` son llamados al render-time y
 *     evitamos forzar un reload reactivo de toda la app.
 *
 * v3.5.0: el refactor a JSON facilita futuras traducciones (un
 * tercero puede editar JSON sin tocar TypeScript) y separa los
 * datos del código de runtime — un cambio de string ya no requiere
 * recompilar la lógica de t().
 */

import esDict from "./i18n/es.json";
import enDict from "./i18n/en.json";

export type Locale = "es" | "en";
export type LocaleSetting = Locale | "auto";

/** Key del localStorage usado por preloadLocale / persistLocale. */
const LS_KEY = "simfleet.locale";

const DICTIONARIES: Record<Locale, Record<string, string>> = {
  es: esDict as Record<string, string>,
  en: enDict as Record<string, string>,
};

/** Resuelve `"auto"` contra `navigator.language` ("es-ES" → "es",
 *  "en-US" → "en", anything else → "en"). */
export function resolveLocale(setting: LocaleSetting): Locale {
  if (setting === "es" || setting === "en") return setting;
  if (typeof navigator !== "undefined") {
    const lang = navigator.language.toLowerCase();
    if (lang.startsWith("es")) return "es";
  }
  return "en";
}

let currentLocale: Locale = "en";

export function setActiveLocale(setting: LocaleSetting): Locale {
  currentLocale = resolveLocale(setting);
  return currentLocale;
}

export function getActiveLocale(): Locale {
  return currentLocale;
}

/**
 * (v3.4.0) Cargado SÍNCRONO del locale antes del primer render.
 *
 * Llamado desde `main.tsx` justo antes de `ReactDOM.createRoot`.
 * Lee `localStorage[LS_KEY]` — escrito por `persistLocale()` cada
 * vez que el usuario cambia idioma (también en el bootstrap del
 * store). Si no hay valor guardado, cae a `"auto"` y resuelve
 * contra `navigator.language`.
 *
 * **Por qué localStorage y no la DB:** la DB de Tauri es async; en
 * el momento en que React renderiza por primera vez el bootstrap
 * aún no terminó. Con localStorage tenemos lectura síncrona y
 * el shell entero (tabs, splash) ya arranca en el idioma correcto.
 */
export function preloadLocale(): Locale {
  let stored: string | null = null;
  try {
    stored =
      typeof localStorage !== "undefined" ? localStorage.getItem(LS_KEY) : null;
  } catch {
    // SecurityError en algunos contextos (iframe sandbox, etc).
    stored = null;
  }
  const setting: LocaleSetting =
    stored === "es" || stored === "en" || stored === "auto" ? stored : "auto";
  return setActiveLocale(setting);
}

/**
 * (v3.4.0) Espeja el setting de idioma al localStorage. Llamado por
 * la store cada vez que `setLanguage()` se ejecuta y al final del
 * bootstrap (para que reflectee lo que la DB tenga guardado, en
 * caso de que el usuario instale en otro PC con sync de cloud).
 */
export function persistLocale(setting: LocaleSetting): void {
  try {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(LS_KEY, setting);
    }
  } catch {
    // Ignorar — el usuario seguirá viendo el locale resuelto en memoria
    // hasta el restart. La DB sigue siendo source of truth.
  }
}

/** Lookup con fallback EN → key. Acepta `args` opcional para
 *  interpolación simple `{name}` → value. */
export function t(key: string, args?: Record<string, string | number>): string {
  const dict = DICTIONARIES[currentLocale] ?? DICTIONARIES.en;
  let value = dict[key] ?? DICTIONARIES.en[key] ?? key;
  if (args) {
    for (const [k, v] of Object.entries(args)) {
      value = value.replace(`{${k}}`, String(v));
    }
  }
  return value;
}
