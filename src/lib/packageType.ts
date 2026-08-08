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
 *  tráfico AI, GSX-extras, FS2Crew, Navigraph, SimBrief, AICopilot,
 *  Flow scripts…). */
const UTILITY_RE =
  /\b(fsltl|fs2crew|raaspro|aicopilot|ai\s*copilot|gsx[\s-]*(?:cobus|profile|fuel|hydrant|catering|menzies|haeco)|simbrief|navigraph|tablet|efb|companion|toolbar|enhancement|improver|plugin|module|tweak|flow\s*(?:pro|scripts?)|command\s*center|truflite|toliss|gauge|wxr|weather\s*radar)\b/i;

/** Keywords de vehículos/objetos no-avión → MISC. */
const VEHICLE_RE =
  /\b(driveable|driver|car\b|cars\b|vehicle|bus(?:es)?|truck|boat|train)\b/i;

/** Keywords de sonido / efectos visuales → MISC. `soundset` cubre los
 *  packs de Boris Audio Works (xbaw-*-soundset-*). */
const SOUND_RE =
  /\b(sound\s*set|soundset|soundpack|sound\s*pack|audio\s*pack|immersion|vfx|boris\s*audio)\b/i;

/** Keywords de modificación de texturas / cabina → MISC (no es avión
 *  aunque declare AIRCRAFT). "downscaled textures", "improve fps",
 *  "new cockpit", "rev"/"revisited" de texturas. */
const TEXTURE_MOD_RE =
  /\b(downscaled|improve\s*fps|new\s*cockpit|cockpit\s*texture|texture\s*(?:pack|mod|overhaul)|retexture|4k\s*texture|cockpit\s*overhaul)\b/i;

/** Señales TEXTUALES explícitas de livery (no deps>0, que es engañoso). */
function looksLikeLivery(p: CommunityPackage): boolean {
  const t = `${p.title} ${p.folderName}`;
  if (/\b(liver(?:y|ies)|repaint|skin|paintkit|paint\s*kit)\b/i.test(t))
    return true;
  // Matrículas reales tipo ICAO: prefijo de 1-2 LETRAS, guion, y una LETRA
  // seguida de 1-4 alfanuméricos (G-DHLV, D-AIMA, EC-NTO, PR-XMM, XA-MAG…).
  // (v7.2.1) Exigimos que tras el guion venga una LETRA para NO confundir
  // designaciones de MODELO (MD-11, DC-10, CRJ-900, 737-800) con matrículas —
  // eso mandaba el TFDi MD-11 a LIVERY. Los modelos llevan DÍGITOS tras el guion.
  if (/\b[A-Z]{1,2}-[A-Z][A-Z0-9]{1,4}\b/.test(p.title)) return true;
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

/** (v4.28.0) Diagnóstico estructural del paquete a partir de
 *  SimObjects/Airplanes + aircraft.cfg. Lo que vimos del FS real es
 *  la verdad absoluta:
 *
 *   · **AIRCRAFT (avión completo)**: tiene containers propios bajo
 *     `SimObjects/Airplanes/<X>` Y todos los `base_container` que
 *     declara apuntan a UNO DE SUS PROPIOS containers (self-contained).
 *   · **LIVERY**: tiene container propio CON `aircraft.cfg` que
 *     declara `base_container` apuntando a OTRO paquete (no a sí
 *     mismo). Pista de la especificación oficial: si hay
 *     `base_container = "..\Asobo_A320"` el avión 3D vive en Asobo,
 *     este pack es solo texturas.
 *   · **MOD/COCKPIT/SOUND**: NO tiene containers propios. Modifica
 *     archivos del avión base (panel, sonido, lights) y por eso
 *     declara `content_type=AIRCRAFT` con `deps>0` aunque no aporte
 *     avión — esos van a MISC/INSTRUMENT por keyword. */
export function diagnosePackage(p: CommunityPackage): {
  isLivery: boolean;
  liveryReason: string | null;
  baseContainer: string | null;
  hasOwnAirplane: boolean;
} {
  const dirs = parseJsonArray(p.simobjectDirsJson).map((s) => s.toLowerCase());
  const bases = parseJsonArray(p.baseContainersJson);
  const ownSet = new Set(dirs);
  const baseOutside = bases.find((b) => !ownSet.has(b.toLowerCase()));
  const ct = (p.contentType ?? "").toUpperCase().trim();
  // Señal #1 (la canónica del SDK): aircraft.cfg con base_container
  // apuntando fuera → livery sí o sí.
  if (baseOutside) {
    return {
      isLivery: true,
      liveryReason: `aircraft.cfg base_container = "${baseOutside}" → otro paquete`,
      baseContainer: baseOutside,
      hasOwnAirplane: false,
    };
  }
  // Señal #2: manifest declara LIVERY explícitamente.
  if (["LIVERY", "PAINT", "REPAINT", "PAINTKIT", "TEXTURE"].includes(ct)) {
    return {
      isLivery: true,
      liveryReason: 'manifest.json content_type = "LIVERY"',
      baseContainer: null,
      hasOwnAirplane: dirs.length > 0,
    };
  }
  return {
    isLivery: false,
    liveryReason: null,
    baseContainer: null,
    hasOwnAirplane: dirs.length > 0,
  };
}

function parseJsonArray(raw: string | undefined): string[] {
  if (!raw) return [];
  try {
    const v = JSON.parse(raw);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

export function derivedType(p: CommunityPackage): DerivedType {
  const ct = p.contentType?.toUpperCase().trim() ?? "";
  const hay = `${p.title} ${p.folderName}`;

  // 1) Library packs (AIRAC, night lights, replacements) marcados ya
  //    por el backend → MISC.
  if (p.isLibraryPack) return "MISC";

  // 2) UTILITIES por keyword (FSLTL/FS2Crew/AICopilot/Flow/GSX-extras).
  //    El usuario: "AICopilot FBW A320 debe ser other con tag 3rd party".
  if (UTILITY_RE.test(hay)) return "INSTRUMENT";

  // 3) Vehículos no-avión (Library Driveable Car).
  if (VEHICLE_RE.test(hay)) return "MISC";

  // 4) Sonido (BAW soundsets, Boris Audio Works).
  if (["SOUND", "SOUNDPACK", "SOUND-PACK", "MUSIC"].includes(ct)) return "MISC";
  if (SOUND_RE.test(hay)) return "MISC";

  // 5) SCENERY no-airport → MISC.
  if (ct === "SCENERY") return "MISC";

  // 6) INSTRUMENTS explícitos del manifest. Los manifests reales de MSFS
  //    usan el plural "INSTRUMENTS" (EFB, gauges); aceptamos ambas formas.
  //    "TOOL" son companion apps/herramientas (SimBridge de FBW) → Utilities.
  if (ct === "INSTRUMENT" || ct === "INSTRUMENTS" || ct === "TOOL")
    return "INSTRUMENT";

  // 7) base_container apuntando FUERA ⇒ LIVERY (spec MSFS).
  const diag = diagnosePackage(p);
  if (diag.isLivery) return "LIVERY";

  // 8) (v7.2.2) SEÑAL DEFINITIVA DE AVIÓN — `hasFlightModel`: el paquete trae
  //    su MODELO DE VUELO (flight_model.cfg / .air) en algún container
  //    SimObjects/Airplanes. Un avión de verdad SIEMPRE lo trae, INCLUSO los
  //    ENCRIPTADOS (PMDG, Fenix, iniBuilds). Las liveries y los mods NUNCA lo
  //    traen — heredan el vuelo del avión base, aunque declaren
  //    content_type=AIRCRAFT.
  //    NO usamos `hasOwnModel` (geometría .gltf): daba FALSOS NEGATIVOS (los
  //    encriptados no exponen .gltf → borraba PMDG/Fenix de Aircraft) y FALSOS
  //    POSITIVOS (un mod de CABINA trae geometría de asientos/revistas y un mod
  //    de luces/HUES referencia el container, sin ser aviones).
  if (ct === "AIRCRAFT") {
    if (p.hasFlightModel) return "AIRCRAFT"; // trae modelo de vuelo → avión real
    // content_type=AIRCRAFT SIN modelo de vuelo → NO es un avión: es una livery
    // o un mod (zHUES cabina, B748-ImproveLights luces, A320 Cabin). Orden:
    //  1. Señal de LIVERY (aerolínea/matrícula/"livery"; el base_container-fuera
    //     ya se capturó en el paso 7) → LIVERY. Un repintado "weathered" de
    //     aerolínea entra acá.
    if (looksLikeLivery(p)) return "LIVERY";
    //  2. Todo lo demás que declara AIRCRAFT sin modelo de vuelo es un MOD
    //     (cabina/luces/texturas) o contenido mal etiquetado → MISC.
    return "MISC";
  }

  // 9) Texturas/mods por keyword aunque no declaren AIRCRAFT.
  if (TEXTURE_MOD_RE.test(hay)) return "MISC";

  // 10) Señales textuales de livery (manifest mal declarado).
  if (looksLikeLivery(p)) return "LIVERY";

  // 11) Sin content_type declarado: si trae su MODELO DE VUELO
  //     (flight_model.cfg / .air) es un avión de verdad (manifest ausente/
  //     ilegible pero estructura de avión presente). Ya NO promovemos por
  //     hasOwnModel (falla en encriptados y en mods de cabina) ni por regex de
  //     nombre (un título con "A320"/"737" sin vuelo es una livery o un mod).
  if (p.hasFlightModel) return "AIRCRAFT";
  if (ct === "MISC") return "MISC";
  return "UNKNOWN";
}

/** (v4.31.0) ¿Es contenido 3rd-party no estándar? Criterio del
 *  usuario: "addons sin manifest son 3rd party por lógica". Lo
 *  consume la UI para el badge. */
export function is3rdParty(p: CommunityPackage): boolean {
  return p.hasManifest === false;
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
/** (v5.0.0) Paquete de MUESTRA / plantilla del SDK de MSFS, no un addon
 *  real. El SDK crea proyectos con el autor placeholder "My Company" y un
 *  aeropuerto de ejemplo (p. ej. ICAO ficticio "SIKK" / "Calciolândia").
 *  El usuario suele tener uno olvidado en Community tras usar DevMode.
 *  No debe contar como aeropuerto en el Map ni en el Link Map. */
export function isSdkSamplePackage(p: CommunityPackage): boolean {
  const creator = (p.creator ?? "")
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "");
  return creator === "mycompany";
}

export function isAirport(p: CommunityPackage): boolean {
  return (
    isScenery(p) &&
    !!p.icao &&
    p.latitude !== null &&
    p.longitude !== null &&
    !isSdkSamplePackage(p) &&
    !(p.isLibraryPack ?? looksLikeLibraryPack(p))
  );
}

/** Todo lo no-airport (incluyendo SCENERY mods sin ICAO resoluble) →
 *  pestaña Addons. */
export function isAddon(p: CommunityPackage): boolean {
  return !isAirport(p);
}
