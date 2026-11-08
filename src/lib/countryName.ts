import { getActiveLocale } from "./i18n";

// (v4.11.0) ISO-2 → nombre de país legible con la API nativa del navegador, sin
// dataset extra, en el idioma de la app.
let REGION_DN: Intl.DisplayNames | null | undefined;

export function countryName(iso?: string | null): string | null {
  if (!iso) return null;
  try {
    if (REGION_DN === undefined) {
      REGION_DN = new Intl.DisplayNames([getActiveLocale()], { type: "region" });
    }
    return REGION_DN?.of(iso.toUpperCase()) ?? iso;
  } catch {
    return iso;
  }
}
