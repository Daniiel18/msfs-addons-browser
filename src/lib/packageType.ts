import type { CommunityPackage } from "./types";

/**
 * Tipo derivado del paquete a partir del manifest. Usamos esto en
 * lugar del `contentType` crudo porque MSFS no distingue liveries
 * de aircraft completos — ambos son `AIRCRAFT`. Para separarlos
 * miramos señales explícitas (nombre, keywords, content_type).
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

// ---------------------------------------------------------------------------
// (v4.27.0) Heurísticas rebalanceadas — reportes de clasificación errónea:
//   · FBW A32NX / FBW A380X / Salty 747-8 → declaran `AIRCRAFT` con
//     `dependencies>0` y mi heurística vieja los mandaba a LIVERY por
//     deps>0 (deps>0 NO implica livery: muchos aviones derivan del
//     Asobo base). Ahora deps>0 sola NO basta; pedimos señal textual.
//   · FSLTL Traffic Base, Library Driveable Car → declaran AIRCRAFT
//     pero no son aviones (tráfico AI / coche). UTILITIES y MISC por
//     keywords ANTES de aceptar el content_type.
//   · GSX Cobus, FS2Crew Command → manifest dice LIVERY/null pero son
//     utilities — interceptados por keyword.
// ---------------------------------------------------------------------------

/** Keywords que delatan UTILITY/INSTRUMENT (panel, EFB, mod, tweak,
 *  tráfico AI, GSX-extras, FS2Crew, Navigraph, SimBrief…). */
const UTILITY_RE =
  /\b(fsltl|fs2crew|raaspro|gsx[\s-]*(?:cobus|profile|fuel|hydrant|catering|menzies|haeco)|simbrief|navigraph|tablet|efb|companion|toolbar|enhancement|improver|plugin|module|tweak|flow\s*pro|command\s*center|truflite|toliss|gauge|wxr|weather\s*radar)\b/i;

/** Keywords de vehículos/objetos no-avión → MISC. */
const VEHICLE_RE =
  /\b(driveable|driver|car\b|cars\b|vehicle|bus(?:es)?|truck|boat|train)\b/i;

/** Keywords de sonido / efectos visuales → MISC. */
const SOUND_RE = /\b(soundpack|sound\s*pack|audio\s*pack|immersion|vfx)\b/i;

/** Señales TEXTUALES explícitas de livery (no deps>0, que es engañoso). */
function looksLikeLivery(p: CommunityPackage): boolean {
  const t = `${p.title} ${p.folderName}`;
  if (/\b(liver(?:y|ies)|repaint|skin|paintkit|paint\s*kit)\b/i.test(t))
    return true;
  // Matrículas reales: G-BAEK, D-AIxx, EC-NTO, B-XXXX, JA8089, N12345…
  if (/\b[A-Z]{1,3}-[A-Z0-9]{2,6}\b/.test(p.title)) return true;
  if (/\bN\d{1,5}[A-Z]{0,2}\b/.test(p.title)) return true;
  // Aerolíneas: keywords genéricas + lista corta de operadores frecuentes.
  if (
    /\b(Airlines?|Airways|Aviation|Air\s+Lines|Cargo|Express|Charter|Connection|Holidays|Worldwide|Linhas|Lineas?|Aerol[ií]neas?)\b/i.test(
      p.title,
    )
  )
    return true;
  if (
    /\b(Lufthansa|Iberia|Vueling|Ryanair|EasyJet|British\s+Airways|KLM|Delta|United|American|Southwest|Alaska|JetBlue|Spirit|Frontier|Norwegian|Wizz|TUI|TAP|Aeromexico|LATAM|Avianca|Qatar|Emirates|Etihad|Singapore|Cathay|ANA|JAL|China\s+Eastern|Air\s+France|Aeroflot|Turkish|Aer\s+Lingus|Finnair|SAS|Swiss|Austrian|Brussels|Eurowings|Condor|TAROM|S7|Saudia|Qantas|Virgin|Air\s+Canada|WestJet|Hawaiian|FedEx|UPS|DHL)\b/i.test(
      p.title,
    )
  )
    return true;
  return false;
}

/** Detector estricto de "esto es un avión": folder con `aircraft` o
 *  título con modelo conocido. Llamado SOLO si no hay señales de
 *  livery — sino un livery de A350 caería como aircraft. */
function looksLikeAircraft(p: CommunityPackage): boolean {
  // El folder con `aircraft` es la señal más fuerte: PMDG, FBW,
  // iniBuilds, Fenix, etc. siguen `<vendor>-aircraft-<model>`.
  if (/\baircraft\b/i.test(p.folderName)) return true;
  const aircraftModelRegex =
    /\b(a3(?:1[89]|2[01]|30|40|50|80)|b73[6789]|b74[78]|b75[7]|b76[7]|b77[7]|b78[7]|crj|md[-_ ]?(?:11|80|90)|tbm[-_ ]?9(?:30|40)|c1?7[2358]|c20[8]|atr[-_ ]?(?:42|72)|q400|dh[c]?[-_ ]?8|cj4|king\s*air|cessna|piper|mooney)\b/i;
  if (aircraftModelRegex.test(p.title)) return true;
  if (aircraftModelRegex.test(p.folderName)) return true;
  if (/\b(aircraft|airplane|jet|airliner)\b/i.test(p.title)) return true;
  return false;
}

export function derivedType(p: CommunityPackage): DerivedType {
  const ct = p.contentType?.toUpperCase().trim() ?? "";
  const hay = `${p.title} ${p.folderName}`;

  // 1) Library packs (AIRAC, night lights, replacements) marcados ya
  //    por el backend — los mandamos a MISC para que no contaminen las
  //    categorías visibles del grid.
  if (p.isLibraryPack) return "MISC";

  // 2) UTILITIES por keyword ANTES de cualquier otra cosa — FSLTL/
  //    FS2Crew/GSX-Cobus declaran a veces AIRCRAFT/LIVERY/null y se
  //    colaban donde no debían.
  if (UTILITY_RE.test(hay)) return "INSTRUMENT";

  // 3) Vehículos no-avión.
  if (VEHICLE_RE.test(hay)) return "MISC";

  // 4) SCENERY no-airport (las airport ya se fueron al Mapa) — son
  //    replacements, jetways sueltos, etc. → MISC.
  if (ct === "SCENERY") return "MISC";

  // 5) Sonido / VFX.
  if (["SOUND", "SOUNDPACK", "SOUND-PACK", "MUSIC"].includes(ct)) return "MISC";
  if (SOUND_RE.test(hay)) return "MISC";

  // 6) INSTRUMENTS y LIVERY explícitos del manifest.
  if (ct === "INSTRUMENT") return "INSTRUMENT";
  if (["LIVERY", "PAINT", "REPAINT", "PAINTKIT", "TEXTURE"].includes(ct))
    return "LIVERY";

  // 7) Señales textuales de livery (más fiables que el manifest, que
  //    muchas liveries de la comunidad ni declaran).
  if (looksLikeLivery(p)) return "LIVERY";

  // 8) AIRCRAFT declarado — ya descartado livery por (7).
  if (ct === "AIRCRAFT") return "AIRCRAFT";

  // 9) Sin content_type — corremos detector de aircraft (cubre packs
  //    sin manifest claro). Si nada calza, queda UNKNOWN.
  if (looksLikeAircraft(p)) return "AIRCRAFT";
  if (ct === "MISC") return "MISC";
  return "UNKNOWN";
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
 * Estricto: solo SCENERY, sin fallbacks.
 */
export function isScenery(p: CommunityPackage): boolean {
  return (p.contentType ?? "").trim().toUpperCase() === "SCENERY";
}

/** Detector local de library pack — fallback para modo demo.
 *  El backend (Rust `is_library_pack`) es la fuente de verdad. */
function looksLikeLibraryPack(p: CommunityPackage): boolean {
  const t = `${p.title} ${p.folderName}`.toLowerCase();
  return /\b(librar(?:y|ies)|developers?\s*pack|object\s*pack|asset\s*pack|jetways?|vehicles?|vehicle\s*pack|vegetation|trees?|grass|autogen|landmark\s*pack|asobo\s*objects?|simobjects?|sdk|placeholder|fsdreamteam-gsx|gsx[\s-]*pro|gsx\s*world)\b/i.test(
    t,
  );
}

/**
 * SCENERY + aeropuerto real (ICAO en airports + coordenadas) + no es
 * library. Lo que pinta el mapa y la sidebar.
 */
export function isAirport(p: CommunityPackage): boolean {
  return (
    isScenery(p) &&
    !!p.icao &&
    p.latitude !== null &&
    p.longitude !== null &&
    !(p.isLibraryPack ?? looksLikeLibraryPack(p))
  );
}

/** Todo lo no-airport (incluyendo SCENERY mods sin ICAO resoluble) →
 *  pestaña Addons. */
export function isAddon(p: CommunityPackage): boolean {
  return !isAirport(p);
}
