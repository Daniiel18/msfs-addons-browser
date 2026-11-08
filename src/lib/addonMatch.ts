import type { Addon, CommunityPackage, DownloadMethod } from "./types";
import { isNewer } from "./version";

/**
 * Cuándo una ficha del catálogo es el addon que tienes instalado. Lo usan el
 * «Instalado» de Buscar, la lista de Reinstalar y el asistente de Actualizar,
 * para que los tres entiendan lo mismo por «este addon».
 */

/**
 * Normaliza un título para compararlo con el folder/title del
 * manifest. Quita versión, sufijos `MSFS 2020`, paréntesis con
 * etiquetas tipo `(Merged)` / `(Converted)` para usarlas como
 * señal pero sin que rompan el match exacto.
 */
export function normalizeTitle(s: string): string {
  return s
    .toLowerCase()
    .replace(/\sv\d+(?:\.\d+)+\b.*$/i, "")
    .replace(/\bmsfs\s*\d{4}(?:\/\d+)?\b/gi, "")
    .replace(/[\s ]+/g, " ")
    .trim();
}

/** (v5.4.1) Colapsa sub-marcas de desarrollador a un token canónico
 *  (solo alfanuméricos). Algunas casas publican la escenografía bajo otra
 *  marca: el manifest trae "iniScene" pero el catálogo lista "iniBuilds" —
 *  misma casa. Sin esto KLAX (iniScene) no se marcaba instalado. */
export function canonDev(s: string | null | undefined): string {
  const n = (s ?? "").replace(/[^a-z0-9]/gi, "").toLowerCase();
  if (n === "iniscene") return "inibuilds";
  return n;
}

/** True si dos cadenas (creator del manifest y developer del
 *  catálogo) corresponden plausiblemente al mismo dueño.
 *  Substring case-insensitive — tolera "Aerosoft" vs "Aerosoft GmbH". */
export function sameCreator(a: string | null | undefined, b: string | null | undefined): boolean {
  const aa = canonDev(a);
  const bb = canonDev(b);
  if (!aa || !bb) return false;
  return aa.includes(bb) || bb.includes(aa);
}

/** Extrae identificadores de modelo de aeronave del texto.
 *  Cubre Airbus (A320/A330/A350/A380…), Boeing (737/747/757/767/777/787),
 *  variantes con sufijos (-200, -300ER, -800), y modelos GA/regionales
 *  comunes (CRJ, ATR, MD-11, DC-6, TBM930, C172).
 *
 *  El usuario reportó: "tengo un A350 instalado, busco A350 en
 *  Simplaza, no me marca instalado". Causa: el catálogo de
 *  Simplaza decía "Airbus A350" pero el manifest del paquete
 *  decía "iniBuilds A350-900" — el substring de título no
 *  coincidía. Extrayendo el modelo común (A350) ambos hacen match.
 */
function aircraftModels(text: string): string[] {
  const out: string[] = [];
  // (25/10) El carguero: "777-Freighter" (aerohub) es el "777F" del manifest
  // de PMDG; si no, quedaba como "777" a secas y casaba con el 777-200ER.
  text = text.replace(/\b(77\d)[\s-]?freighter\b/gi, "$1F");
  const re =
    /\b(A\d{3}(?:-?\d{3})?|B\d{3}(?:-?\d{3}[A-Z]{0,3})?|73[6-9](?:-?\d{3})?|74[478](?:-?\d{1,3})?|76\d|77\d(?:F|-?\d{3}[A-Z]{0,3})?|78\d|CRJ\d?\d?|ATR-?\d{2}\d?|MD-?\d{2}|DC-?\d|TBM\d{3}|C\d{3,4}|CJ\d|Q\d{3})\b/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    // Normalizamos: "A350-900" → "A350900"; "737-800" → "737800";
    // sin guiones para que el match sea robusto (la fuente y el
    // manifest pueden o no incluir el guión).
    out.push(m[1].toUpperCase().replace(/-/g, ""));
  }
  return out;
}

/** True si ambos textos comparten al menos un identificador de
 *  modelo de aeronave (A350, B777, etc.). Tolerante a sufijos:
 *  "A350" matchea con "A350-900" porque tras normalización ambos
 *  contienen "A350" como prefijo. */
function shareAircraftModel(a: string, b: string): boolean {
  const am = aircraftModels(a);
  if (am.length === 0) return false;
  const bm = aircraftModels(b);
  if (bm.length === 0) return false;
  for (const x of am) {
    for (const y of bm) {
      // Match exacto o por prefijo (A350 ⊂ A350900).
      if (x === y || x.startsWith(y) || y.startsWith(x)) return true;
    }
  }
  return false;
}

/** True si el nombre delata un addon "categoría" (livery, sound,
 *  preset, paint, etc.) — usado para el filtro asimétrico que
 *  evita marcar como "instalado/update disponible" un livery pack
 *  cuando lo que el usuario tiene es el avión base. */
function isCategoryAddonName(s: string): boolean {
  return /\b(liver(y|ies)|paint|texture|sound\s*pack|sound\b|preset|profile|config|tweak|enhancement|\bmod\b|\bpack\b|repaint)/i.test(
    s,
  );
}

/** El paquete instalado que corresponde a esta ficha del catálogo. */
export function findInstalled(
  addon: Addon,
  packages: CommunityPackage[],
): CommunityPackage | undefined {
  // Match consistente con el detector backend:
  //   A. SCENERY → ICAO + creator/developer.
  //   B. AIRCRAFT/livery/etc → creator/developer + título (substring).
  //
  // Un match estricto por creator evita falsos "Instalado" cuando
  // hay dos sceneries del mismo ICAO de devs distintos (Aerosoft
  // EBBR + JustSim EBBR — solo el realmente instalado debe marcarse).

  let exactMatch: CommunityPackage | undefined;

  // Camino A — ICAO + creator
  if (addon.icao) {
    const target = addon.icao.toUpperCase();
    const sameIcao = packages.filter(
      (p) => p.icao && p.icao.toUpperCase() === target,
    );
    if (sameIcao.length > 0 && addon.developer) {
      exactMatch = sameIcao.find((p) => sameCreator(p.creator, addon.developer));
    }
    if (!exactMatch) {
      // Fallback por título normalizado — útil cuando el manifest
      // del paquete instalado no expone creator parseable. PERO:
      // si ambos lados tienen developer y son DISTINTOS, NO es
      // match. El bug reportado por el usuario: tenía RCTP de
      // ACO Design Studio v1.2.0 instalado, y el card del RCTP
      // de Double T en el catálogo aparecía como "Instalado" sólo
      // porque normalizeTitle(ACO) === normalizeTitle(Double T).
      // Asimetría: si los devs existen y difieren → distintos
      // productos del mismo aeropuerto.
      const addonNorm = normalizeTitle(addon.name);
      exactMatch = sameIcao.find((p) => {
        if (normalizeTitle(p.title) !== addonNorm) return false;
        if (
          addon.developer &&
          p.creator &&
          !sameCreator(p.creator, addon.developer)
        ) {
          return false;
        }
        return true;
      });
    }
  }

  // Camino B — creator + (título substring O modelo de aeronave
  // compartido). El match por modelo cubre el caso "iniBuilds –
  // Airbus A350" del catálogo vs "iniBuilds A350-900" del manifest:
  // el creator matchea por substring (iniBuilds ⊂ iniBuilds Limited)
  // y ambos contienen el modelo A350.
  //
  // **Filtro asimétrico anti-categoría** (v0.1.10, mismo principio
  // que el SQL en compute_available): si el addon del catálogo es
  // de otra "categoría" (livery, sound, preset, paint, mod, pack)
  // pero el paquete instalado NO lo es, NO matchea — un avión
  // base no se "actualiza" con un livery pack. Bug reportado
  // por usuario: tenía PMDG 737-800 base y aparecía como UPDATE
  // disponible "Boeing 737-700/800/900 Liveries by PMDG".
  if (!exactMatch && addon.developer) {
    exactMatch = packages.find((p) => {
      if (!sameCreator(p.creator, addon.developer)) return false;
      const a = addon.name.toLowerCase();
      const b = p.title.toLowerCase();
      const titlesOverlap =
        a.includes(b) ||
        b.includes(a) ||
        shareAircraftModel(addon.name, p.title);
      if (!titlesOverlap) return false;
      // Asimetría: a (catálogo) contiene categoría, b (instalado) no → reject.
      if (isCategoryAddonName(a) && !isCategoryAddonName(b)) return false;
      return true;
    });
  }

  // **No hay Camino C para AIRCRAFT/livery.** Antes intentábamos
  // detectar por modelo sólo (sin creator) para Simplaza — pero eso
  // daba falsos positivos masivos: buscar "737" marcaba como instalados
  // todos los addons de 737 (FS2Crew SOP, MSFS Wear, repaints…)
  // porque el usuario tenía PMDG 737-800 base. La heurística por
  // modelo sin creator es inherentemente ambigua: dos productos
  // distintos pueden ser para el mismo avión.
  //
  // **(v3.7.0 fix O1) Camino D — SCENERY con ICAO único**:
  // si el addon es de un aeropuerto (tiene ICAO) y el usuario tiene
  // **exactamente UN** paquete con ese ICAO en Community, lo marcamos
  // como instalado aunque el catálogo no exponga developer parseable
  // (caso KATL Imagine Simulation: el catálogo devolvía addon.developer
  // null y title largo, los Caminos A/B fallaban). El riesgo de falso
  // positivo es bajo: pilotos rara vez tienen 2 sceneries del mismo
  // aeropuerto instalados al mismo tiempo. Si tienen 2+, el match
  // estricto sigue activo (los multi-ICAO se manejan en Camino A).
  if (!exactMatch && addon.icao) {
    const target = addon.icao.toUpperCase();
    const sameIcao = packages.filter(
      (p) => p.icao && p.icao.toUpperCase() === target,
    );
    if (sameIcao.length === 1) {
      const only = sameIcao[0];
      // (v5.0.0) BLINDAJE por creador: el Camino D existía para el caso
      // de catálogo SIN developer parseable. Pero si el catálogo SÍ
      // declara un developer y NO coincide con el del paquete instalado,
      // es OTRO producto del mismo aeropuerto — no lo marcamos. Bug
      // reportado: con Aerosoft ENGM instalado, las fichas de Orbx y
      // JustSim (mismo ICAO) aparecían como "Installed/Update".
      const devConflict =
        !!addon.developer &&
        !!only.creator &&
        !sameCreator(only.creator, addon.developer);
      if (!devConflict) {
        exactMatch = only;
      }
    }
  }

  return exactMatch;
}

/** (05/11) ¿Es de ese desarrollador? Por el manifest o por la casa que va en el
 *  nombre de la carpeta, que no siempre coinciden. */
export function isFromDeveloper(pkg: CommunityPackage, developer: string): boolean {
  const dev = canonDev(developer);
  return sameCreator(pkg.creator, developer) || (!!dev && canonDev(pkg.folderName).startsWith(dev));
}

/** (04/11) ¿Esta ficha del catálogo es el addon instalado en `pkg`? La regla
 *  de arriba, y además la casa que va en el nombre de la carpeta: el manifest a
 *  veces firma otro (LIMESIM en el EDDB de Aerosoft, WSM en el SKBO de WingSim). */
export function isSameAddon(addon: Addon, pkg: CommunityPackage): boolean {
  if (findInstalled(addon, [pkg])) return true;
  const dev = canonDev(addon.developer);
  return (
    !!dev &&
    !!addon.icao &&
    addon.icao.toUpperCase() === pkg.icao?.toUpperCase() &&
    canonDev(pkg.folderName).startsWith(dev)
  );
}

/** Una fila de la lista de Reinstalar: una ficha con sus enlaces ordenados. */
export interface ReinstallChoice {
  addon: Addon;
  methods: DownloadMethod[];
}

const METHOD_ORDER: Record<string, number> = { torrent: 0, mirror: 1, direct: 2 };

/** (04/11) Qué ofrece Reinstalar: `own` es este mismo addon y `others` lo demás
 *  que trajo la búsqueda. Una fila por producto y simulador, con su versión más
 *  alta en cada fuente. Antes salía una sola fila por fuente, y en CYOW Roman
 *  Design v1.4.0 tapaba a Simaddons v1.0.1, que ni aparecía. */
export function reinstallChoices(
  matches: Addon[],
  pkg: CommunityPackage,
): { own: ReinstallChoice[]; others: ReinstallChoice[] } {
  const newest = new Map<string, Addon>();
  for (const a of matches) {
    if (a.downloadMethods.length === 0) continue;
    const key = [a.source, canonDev(a.developer), normalizeTitle(a.name), a.simulator].join("|");
    const prev = newest.get(key);
    if (!prev || (a.version && (!prev.version || isNewer(a.version, prev.version)))) {
      newest.set(key, a);
    }
  }
  const choices = [...newest.values()].map((addon) => ({
    addon,
    methods: [...addon.downloadMethods].sort(
      (x, y) => (METHOD_ORDER[x.kind] ?? 9) - (METHOD_ORDER[y.kind] ?? 9),
    ),
  }));
  return {
    own: choices.filter((c) => isSameAddon(c.addon, pkg)),
    others: choices.filter((c) => !isSameAddon(c.addon, pkg)),
  };
}
