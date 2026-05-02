import type { CommunityPackage } from "./types";

/**
 * Tipo derivado del paquete a partir del manifest. Usamos esto en
 * lugar del `contentType` crudo porque MSFS no distingue liveries
 * de aircraft completos — ambos son `AIRCRAFT`. Para separarlos
 * miramos `dependenciesCount`: una livery declara siempre una
 * dependencia hacia el aircraft base.
 *
 * Mantener esto como una única función pura, consumida por las dos
 * vistas (Mapa y Addons) garantiza que los filtros sean coherentes.
 */
export type DerivedType =
  | "SCENERY"
  | "AIRCRAFT"
  | "LIVERY"
  | "INSTRUMENT"
  | "MISC"
  | "UNKNOWN";

/**
 * Heurísticas que sugieren que un paquete es una livery, aunque el
 * manifest no lo declare explícitamente. Las exponemos aparte para
 * reusarlas sobre AIRCRAFT y UNKNOWN sin duplicar código.
 *
 *   · Pista textual: la palabra "livery" aparece en el título.
 *   · Matrícula tipo `LETRA-LETRAS/DÍGITOS` (DAL-N377NW, B-327Y,
 *     EC-NTO, D-AIxx). Es el patrón canónico de una repintura
 *     basada en una matrícula real.
 *   · Aerolínea en el título: "Airlines", "Airways", "Cargo", etc.
 *
 * Estas heurísticas ya estaban aplicadas a AIRCRAFT, ahora las
 * usamos también en UNKNOWN porque el manifest de muchas liveries
 * de la comunidad llega con `content_type` vacío o malformado.
 */
function looksLikeLivery(p: CommunityPackage): boolean {
  if (p.dependenciesCount > 0) return true;
  if (/\blivery\b/i.test(p.title)) return true;
  if (/\blivery\b/i.test(p.folderName)) return true;
  if (/\b[A-Z]{1,3}-[A-Z0-9]{2,6}\b/.test(p.title)) return true;
  if (/\b(Airlines?|Airways|Aviation|Air\s+Lines|Cargo|Express)\b/i.test(p.title))
    return true;
  return false;
}

/**
 * Clasificación basada en el manifest, con fallbacks heurísticos
 * para casos donde el `content_type` no está poblado correctamente.
 */
export function derivedType(p: CommunityPackage): DerivedType {
  const ct = p.contentType?.toUpperCase().trim() ?? "";

  if (ct === "SCENERY") return "SCENERY";

  if (ct === "AIRCRAFT") {
    return looksLikeLivery(p) ? "LIVERY" : "AIRCRAFT";
  }

  if (ct === "INSTRUMENT") return "INSTRUMENT";
  if (ct === "MISC") return "MISC";

  // Sin content_type — antes lo dejábamos en UNKNOWN sin más,
  // pero el usuario reportó que muchas liveries caen aquí. Si el
  // título/folder tiene patrones inconfundibles de livery (matrícula,
  // "Airlines", la palabra "livery"), lo clasificamos como tal en
  // vez de esconderlo bajo UNKNOWN.
  if (!ct && looksLikeLivery(p)) return "LIVERY";
  return "UNKNOWN";
}

/**
 * Estricto: solo SCENERY, sin fallbacks. Útil cuando queremos
 * categorizar un paquete sin importar si tiene coords resueltos.
 */
export function isScenery(p: CommunityPackage): boolean {
  return derivedType(p) === "SCENERY";
}

/**
 * Aún más estricto: SCENERY **+ aeropuerto real** (ICAO en la tabla
 * de aeropuertos, con coordenadas resueltas). Esto es lo que pinta
 * el mapa y lo que aparece en la sidebar — todo lo demás cae en
 * la pestaña Addons. Incluye la validación contra `airports.icao`
 * implícitamente: si lat/lon vienen null es porque el LEFT JOIN del
 * backend no encontró el ICAO en OurAirports, así que no es un
 * aeropuerto base sino un mod (GSX HAECO, Catering, Replacement…).
 */
export function isAirport(p: CommunityPackage): boolean {
  return (
    isScenery(p) &&
    !!p.icao &&
    p.latitude !== null &&
    p.longitude !== null
  );
}

/**
 * Inverso de `isAirport`: todo lo que no es un aeropuerto base va a
 * la pestaña Addons. Esto incluye AIRCRAFT, LIVERY, INSTRUMENT, MISC
 * y SCENERY mods (catering, replacements, sound packs etc.) que no
 * resuelven a un aeropuerto en OurAirports.
 */
export function isAddon(p: CommunityPackage): boolean {
  return !isAirport(p);
}
