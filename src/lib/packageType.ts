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
  // Manifest dice que depende de algo — la pista canónica de
  // livery (depende del aircraft base).
  if (p.dependenciesCount > 0) return true;

  // Palabras explícitas (singular + plural + sinónimos).
  // `\b` cuando hay límite de palabra; el folder lleva guiones y
  // el regex `\b` también los considera bordes.
  const liveryWords = /\b(liver(?:y|ies)|repaint|skin|texture|paintkit|paint\s*kit)\b/i;
  if (liveryWords.test(p.title)) return true;
  if (liveryWords.test(p.folderName)) return true;

  // Matrícula de aeronave: `LETRA(s)-LETRAS/DÍGITOS`. Captura
  // EC-NTO, D-AIxx, B-XXXX, N12345, JA8089... La N-reg US la
  // detecta el fallback de tres-cinco dígitos sin guión.
  if (/\b[A-Z]{1,3}-[A-Z0-9]{2,6}\b/.test(p.title)) return true;
  // N-registry americana: `N` seguida de 1–5 dígitos opcionales y
  // hasta 2 letras. Patrón explícito.
  if (/\bN\d{1,5}[A-Z]{0,2}\b/.test(p.title)) return true;

  // Aerolínea/operador en el título — keywords amplias.
  if (
    /\b(Airlines?|Airways|Aviation|Air\s+Lines|Cargo|Express|Charter|Connection|Air\s+Way|Holidays|Worldwide|Linhas|Lineas?|Aerol[ií]neas?)\b/i.test(
      p.title,
    )
  )
    return true;

  // Aerolíneas comunes — lista corta de nombres frecuentes en
  // packs comunitarios. No exhaustiva pero cubre lo típico que
  // el usuario reportó como UNKNOWN. Case-insensitive.
  if (
    /\b(Lufthansa|Iberia|Vueling|Ryanair|EasyJet|British\s+Airways|KLM|Delta|United|American|Southwest|Alaska|JetBlue|Spirit|Frontier|Norwegian|Wizz|TUI|TAP|Aeromexico|LATAM|Avianca|Qatar|Emirates|Etihad|Singapore|Cathay|ANA|JAL|China\s+Eastern|Air\s+France|Aeroflot|Turkish|Aer\s+Lingus|Finnair|SAS|Swiss|Austrian|Brussels|Eurowings|Condor|TAROM|S7|Saudia|Qantas|Virgin|Air\s+Canada|WestJet|Hawaiian|FedEx|UPS|DHL)\b/i.test(
      p.title,
    )
  )
    return true;

  // Patrón folder: `<aircraft>-<airline>` y similares. Los packs
  // de FlyByWire siguen `flybywire-aircraft-<a32nx|a380x>-<airline>`,
  // PMDG usa `pmdg-737-<airline>`, etc. Si el folder contiene un
  // identificador de aircraft + sufijo de aerolínea conocida.
  if (
    /(?:a32(?:0|9)nx|a380x|a330|a350|737|738|747|757|767|777|787|tbm930|cj4|crj|dh8|q400|atr|c172|c182|c208|md11|md80|md90)[-_](?:livery|airlines?|cargo|express|airways|paint|skin|repaint)/i.test(
      p.folderName,
    )
  )
    return true;

  return false;
}

/**
 * Clasificación basada en el manifest, con fallbacks heurísticos
 * para casos donde el `content_type` no está poblado correctamente.
 *
 * v0.1.14: mucho más permisivo — el usuario reportó demasiados
 * "Sin clasificar" en su lista. Aceptamos content_types no
 * estándar (LIVERY, PAINT, REPAINT, SOUNDPACK, etc.) y siempre
 * corremos `looksLikeLivery`/`looksLikeAircraft` cuando el ct
 * no es SCENERY/INSTRUMENT.
 */
export function derivedType(p: CommunityPackage): DerivedType {
  const ct = p.contentType?.toUpperCase().trim() ?? "";

  if (ct === "SCENERY") return "SCENERY";
  if (ct === "INSTRUMENT") return "INSTRUMENT";

  // Content types no estándar pero comunes en addons de terceros.
  if (["LIVERY", "PAINT", "REPAINT", "PAINTKIT", "TEXTURE"].includes(ct)) {
    return "LIVERY";
  }
  if (["SOUND", "SOUNDPACK", "SOUND-PACK", "MUSIC"].includes(ct)) {
    return "MISC";
  }

  if (ct === "AIRCRAFT") {
    return looksLikeLivery(p) ? "LIVERY" : "AIRCRAFT";
  }

  if (ct === "MISC") return "MISC";

  // Sin content_type O content_type desconocido — corremos las
  // heurísticas en orden:
  //   1. Patrones explícitos de livery (Paintkit, matrícula, etc).
  //   2. Patrones de aircraft (modelo en el nombre/folder).
  //   3. Caer a UNKNOWN sólo si nada calza.
  if (looksLikeLivery(p)) return "LIVERY";
  if (looksLikeAircraft(p)) return "AIRCRAFT";
  return "UNKNOWN";
}

/** Heurística para detectar aircraft sin manifest claro:
 *  · Modelo de avión en el título o folder (A350, 737, CRJ, etc).
 *  · Palabras "aircraft", "plane", "jet" en el título.
 *  Llamada SÓLO después de `looksLikeLivery` para evitar que un
 *  livery de A350 caiga como AIRCRAFT. */
function looksLikeAircraft(p: CommunityPackage): boolean {
  const aircraftModelRegex =
    /\b(a3(?:1[89]|2[01]|30|40|50|80)|b73[6789]|b74[78]|b75[7]|b76[7]|b77[7]|b78[7]|crj|md[-_ ]?(?:11|80|90)|tbm[-_ ]?9(?:30|40)|c1?7[2358]|c20[8]|atr[-_ ]?(?:42|72)|q400|dh[c]?[-_ ]?8|cj4|king\s*air|cessna|piper|mooney)\b/i;
  if (aircraftModelRegex.test(p.title)) return true;
  if (aircraftModelRegex.test(p.folderName)) return true;
  if (/\b(aircraft|airplane|jet|airliner)\b/i.test(p.title)) return true;
  return false;
}

/** True si el thumbnail del paquete es probablemente un placeholder
 *  (creator no se molestó en hacer una imagen real). Lo detectamos
 *  por el TÍTULO — si contiene "NTEST", "Template", "Placeholder",
 *  "Generic" o variantes, no vale la pena cargar el thumbnail (es
 *  el mismo PNG gris en todos). Devolvemos true → renderear el
 *  icono de categoría en su lugar. */
export function looksLikePlaceholderTitle(title: string): boolean {
  return /\b(N?TEST|TEMPLATE|PLACEHOLDER|DEFAULT|SAMPLE|EXAMPLE|GENERIC|TEMP|DEV)\b/i.test(
    title,
  );
}

/**
 * Estricto: solo SCENERY, sin fallbacks. Útil cuando queremos
 * categorizar un paquete sin importar si tiene coords resueltos.
 */
export function isScenery(p: CommunityPackage): boolean {
  return derivedType(p) === "SCENERY";
}

/**
 * (v2.1.0) Detector de "library pack" — paquetes que su manifest
 * declara SCENERY pero en realidad son librerías compartidas, packs
 * de objetos, vegetación, jetways, autogen, etc. No deberían aparecer
 * en el mapa de escenarios aunque su nombre contenga un ICAO casual.
 *
 * Caso reportado por el usuario: "EDHK Lights & Objects Developers
 * Pack" — content_type es SCENERY, contiene "EDHK" como token de 4
 * letras, pero NO es un escenario de aeropuerto, es una librería para
 * desarrolladores. Lo filtramos por palabras clave.
 */
function looksLikeLibraryPack(p: CommunityPackage): boolean {
  const t = `${p.title} ${p.folderName}`.toLowerCase();
  // Palabras que delatan que es una librería / pack de objetos.
  return /\b(librar(?:y|ies)|developers?\s*pack|object\s*pack|asset\s*pack|jetways?|vehicles?|vehicle\s*pack|vegetation|trees?|grass|autogen|landmark\s*pack|asobo\s*objects?|simobjects?|sdk|placeholder)\b/i.test(
    t,
  );
}

/**
 * Aún más estricto: SCENERY **+ aeropuerto real** (ICAO en la tabla
 * de aeropuertos, con coordenadas resueltas) **+ no es una librería**.
 * Esto es lo que pinta el mapa y lo que aparece en la sidebar — todo
 * lo demás cae en la pestaña Addons.
 */
export function isAirport(p: CommunityPackage): boolean {
  return (
    isScenery(p) &&
    !!p.icao &&
    p.latitude !== null &&
    p.longitude !== null &&
    // (v4.24.1) La clasificación la computa el BACKEND (is_library_pack
    // en Rust: AIRAC, night lights, enhancements, excludes…) — la misma
    // que usa el conteo de AIRPORTS del dashboard, así ambos números
    // coinciden siempre. La heurística local queda de fallback para el
    // modo demo (sin backend).
    !(p.isLibraryPack ?? looksLikeLibraryPack(p))
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
