/**
 * (v5.2.9 / v5.3.0) Normaliza el `ATC TYPE` de SimConnect para mostrarlo
 * en el FlightBook, funcionando en MSFS 2020 **y** 2024.
 *
 * - **MSFS 2020**: la SimVar `ATC TYPE` ya viene resuelta (ej. "Boeing",
 *   "Airbus") → la devolvemos tal cual.
 * - **MSFS 2024**: devuelve el token de localización SIN resolver, p.ej.
 *   `ATCCOM.ATC_NAME BOEING.0.text` / `ATCCOM.ATC_NAME AIRBUS.0.text`. El
 *   fabricante está DENTRO del token; lo extraemos y capitalizamos para
 *   mostrar "Boeing"/"Airbus" igual que en 2020.
 * - Otros placeholders sin info útil → null (la UI cae al título del avión).
 */
export function cleanAtcType(s: string | null | undefined): string | null {
  if (!s) return null;
  const t = s.trim();
  if (!t) return null;

  // MSFS 2024: "ATCCOM.ATC_NAME <FABRICANTE>.<n>.text" → extrae el fabricante.
  const m = t.match(/ATC_NAME\s+(.+?)\.\d+\.text\s*$/i);
  if (m) {
    const name = titleCase(m[1].replace(/[._]+/g, " ").trim());
    return name || null;
  }

  // Cualquier otro token de localización que no sepamos parsear → null.
  const lower = t.toLowerCase();
  if (
    lower.includes("atccom") ||
    lower.endsWith(".text") ||
    lower.startsWith("tt:") ||
    lower.startsWith("$$:")
  ) {
    return null;
  }

  // MSFS 2020 / valores ya legibles → tal cual.
  return t;
}

/** "BOEING" → "Boeing", "airbus a320" → "Airbus A320". */
function titleCase(s: string): string {
  return s
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}
