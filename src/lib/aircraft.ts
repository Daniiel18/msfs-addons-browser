/**
 * (v5.2.9) Saneo del `ATC TYPE` de SimConnect.
 *
 * Varios addons (PMDG, iFly, etc.) NO devuelven un tipo limpio ("B738") en
 * la SimVar `ATC TYPE`, sino un token de localización SIN resolver del
 * motor de MSFS, p.ej. `ATCCOM.ATC_NAME BOEING.0.text` o
 * `ATCCOM.ATC_NAME AIRBUS.0.text`. Eso no sirve como nombre de avión y se
 * veía crudo en el FlightBook. Esta función lo descarta (devuelve null)
 * para que la UI caiga al título del avión, que sí es legible.
 */
export function cleanAtcType(
  s: string | null | undefined,
): string | null {
  if (!s) return null;
  const t = s.trim();
  if (!t) return null;
  const lower = t.toLowerCase();
  // Tokens de localización / placeholders típicos que NO son un tipo real.
  if (
    lower.includes("atccom") ||
    lower.endsWith(".text") ||
    lower.startsWith("tt:") ||
    lower.startsWith("$$:")
  ) {
    return null;
  }
  return t;
}
