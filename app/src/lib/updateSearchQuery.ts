/**
 * Deriva un query de búsqueda razonable a partir de una update
 * disponible. La rama AIRCRAFT del SQL en `catalog_versions_for_community`
 * emite `icao = ""` para paquetes no-SCENERY (aviones, liveries, sounds),
 * así que usar `u.icao` directamente como query produce `search("",
 * source)` → 0 resultados ("Sin resultados para """).
 *
 * Heurística:
 *   1. Si `icao` parece un ICAO real (exactamente 4 letras ASCII),
 *      lo usamos tal cual — los catálogos lo soportan bien.
 *   2. Si no, sacamos un keyword del título: primer par de tokens
 *      alfanuméricos ≥ 3 chars, ignorando palabras genéricas tipo
 *      "the" / "and" / "pack" / "for".
 *   3. Si por algún motivo no extraemos nada, devolvemos el título
 *      completo trimmed — peor que un keyword pero mejor que vacío.
 *
 * Lo usan `NotificationsBell` y `UpdateWizard` para no hacer búsquedas
 * vacías que confunden al usuario.
 */
const GENERIC_STOPWORDS = new Set([
  "the",
  "and",
  "for",
  "with",
  "pack",
  "from",
  "this",
  "that",
  "msfs",
  "scenery",
  "airport",
  "aircraft",
  "livery",
  "liveries",
]);

export function looksLikeIcao(s: string): boolean {
  return /^[a-z]{4}$/i.test(s.trim());
}

export function deriveUpdateSearchQuery(icao: string, title: string): string {
  const i = (icao ?? "").trim();
  if (looksLikeIcao(i)) return i;
  const t = (title ?? "").trim();
  if (!t) return "";

  const tokens = t
    .split(/[\s\-_/]+/)
    .map((w) => w.replace(/[^A-Za-z0-9]/g, ""))
    .filter((w) => w.length >= 3 && !GENERIC_STOPWORDS.has(w.toLowerCase()));

  // Primeros dos tokens significativos — suele bastar para
  // discriminar (ej. "iniBuilds A350" o "Fenix A320").
  const picked = tokens.slice(0, 2).join(" ").trim();
  return picked || t;
}
