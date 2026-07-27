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

/** (v6.2.58) Mods de cabina/texturas PMDG/iFly que NO son liveries de
 *  aerolínea (zHUES, "new cockpit", cabina sucia/desgastada). Se quedan en
 *  MISC aunque el paquete sea PMDG/iFly. */
const COCKPIT_MOD_RE =
  /\b(hues|cockpit|panel\s*mod|instrument\s*mod|weathered|dirty|used\s*look|worn)\b/i;

/** (v6.2.58) Livery de PMDG/iFly: un paquete de sólo-texturas (containers
 *  pero SIN modelo propio) que referencia un avión PMDG o iFly es casi
 *  siempre una LIVERY de aerolínea — antes caía en MISC por declarar
 *  content_type=AIRCRAFT sin modelo. Excluimos los mods de cabina. */
function looksLikePmdgIflyLivery(p: CommunityPackage): boolean {
  const t = `${p.title} ${p.folderName}`.toLowerCase();
  if (!/\b(pmdg|ifly)\b/.test(t)) return false;
  if (COCKPIT_MOD_RE.test(t)) return false;
  return true;
}

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

  // 6) INSTRUMENTS explícitos del manifest.
  if (ct === "INSTRUMENT") return "INSTRUMENT";

  // 7) base_container apuntando FUERA ⇒ LIVERY (spec MSFS).
  const diag = diagnosePackage(p);
  if (diag.isLivery) return "LIVERY";

  // 8) (v4.31.0) SEÑAL MAESTRA — ¿tiene geometría 3D propia?
  //    Un AVIÓN real trae model/ con .gltf bajo SimObjects/Airplanes.
  //    Las "texturas de cabina" (zHUES PMDG, A330 New Cockpit) tienen
  //    carpeta SimObjects pero SOLO texture.* — NO model. Declaran
  //    content_type=AIRCRAFT pero NO son aviones → MISC.
  const hasContainers =
    parseJsonArray(p.simobjectDirsJson).length > 0;
  if (ct === "AIRCRAFT") {
    if (p.hasOwnModel) return "AIRCRAFT"; // avión de verdad
    if (hasContainers) {
      // (v6.2.58) Un paquete de sólo-texturas (containers, sin modelo) puede
      // ser una LIVERY de aerolínea, NO un mod de cabina. Lo mandamos a
      // LIVERY si hay señal de livery (aerolínea/matrícula) o si es un livery
      // de PMDG/iFly. Si no, es una mod de cabina/texturas → MISC.
      if (!TEXTURE_MOD_RE.test(hay) && (looksLikeLivery(p) || looksLikePmdgIflyLivery(p)))
        return "LIVERY";
      return "MISC"; // modifica un avión (texturas/cabina)
    }
    // AIRCRAFT sin containers ni modelo: keywords de textura → MISC,
    // si no, lo dejamos como AIRCRAFT (manifest correcto sin SimObjects
    // parseable — raro pero posible).
    if (TEXTURE_MOD_RE.test(hay)) return "MISC";
    return "AIRCRAFT";
  }

  // 9) Texturas/mods por keyword aunque no declaren AIRCRAFT.
  if (TEXTURE_MOD_RE.test(hay)) return "MISC";

  // 10) Señales textuales de livery (manifest mal declarado).
  if (looksLikeLivery(p)) return "LIVERY";

  // 11) Sin content_type: si tiene modelo propio es avión.
  if (p.hasOwnModel) return "AIRCRAFT";
  if (looksLikeAircraft(p)) return "AIRCRAFT";
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
