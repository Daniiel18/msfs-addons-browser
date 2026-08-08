import type { CommunityPackage } from "./types";

/**
 * (v7.2.5) Resolución de la página de liveries de flightsim.to para CUALQUIER
 * avión — siempre devuelve una URL útil.
 *
 * DOS niveles:
 *  1. Página dedicada `https://flightsim.to/liveries/<slug>` para los aviones
 *     conocidos (tabla RESOLVERS de slugs VERIFICADOS contra las páginas
 *     reales). El <slug> es el nombre de marketing del desarrollador + modelo y
 *     flightsim.to es inconsistente (pmdg-boeing-777f PERO pmdg-777-200lr;
 *     Headwind A330-900 = airbus-a330-900 SIN prefijo de marca), por eso se
 *     hardcodean verificados.
 *  2. FALLBACK que funciona para cualquier avión, incluidos los NUEVOS que
 *     agregues: la BÚSQUEDA real de flightsim.to
 *       https://flightsim.to/search?q=<término>&cat=liveries
 *     (el parámetro es `q`, NO `search`; `cat=liveries` acota a la categoría
 *     Liveries; los resultados los pinta el JS del sitio → funciona dentro del
 *     WebView Chromium). La búsqueda es tipo AND por tokens, así que el término
 *     debe ser CORTO y limpio (marca + familia de modelo, sin sufijos de
 *     variante ni ruido de versión) — "Headwind A330-900" da 0 pero "Headwind"
 *     da cientos.
 *
 * Reglas de matching por SUBCADENA sobre el heno (creator+título+carpeta en
 * minúsculas) — NO `\b`, porque los códigos vienen pegados ("737max8", "77w").
 * Para EXTENDER: agrega un resolver a RESOLVERS. Los no cubiertos caen al
 * fallback de búsqueda automáticamente.
 */

const BASE = "https://flightsim.to";
const LIVERIES = `${BASE}/liveries`;

type Resolver = (h: string) => string | null;

const RESOLVERS: { label: string; resolve: Resolver }[] = [
  {
    label: "iFly 737 MAX",
    resolve: (h) => (h.includes("ifly") ? "ifly-boeing-737-max" : null),
  },
  {
    label: "FlightSim Studio E-Jets 170/175",
    resolve: (h) => {
      if (
        h.includes("flightsim studio") ||
        /\bfss\b/.test(h) ||
        h.includes("e17x") ||
        h.includes("e170") ||
        h.includes("e175") ||
        h.includes("e-jet") ||
        h.includes("ejet")
      )
        return "flightsim-studio-e-jets-175";
      return null;
    },
  },
  {
    label: "Fenix A319 / A320 / A321",
    resolve: (h) => {
      if (!h.includes("fenix") && !h.includes("fnx")) return null;
      if (h.includes("321")) return "fenix-simulations-a321";
      if (h.includes("319")) return "fenix-simulations-a319";
      return "fenix-simulations-a320";
    },
  },
  {
    label: "iniBuilds A350 / A320neo",
    resolve: (h) => {
      if (!h.includes("inibuilds")) return null;
      if (h.includes("a350") || h.includes("350")) return "inibuilds-a350";
      if (h.includes("a320") || h.includes("320neo")) return "inibuilds-a320neo";
      return null;
    },
  },
  {
    label: "PMDG 737 / 777 / DC-6 (slug exacto por variante)",
    resolve: (h) => {
      if (!h.includes("pmdg")) return null;
      if (h.includes("dc-6") || h.includes("dc6")) return "pmdg-dc-6";
      // Familia 777
      if (h.includes("77f") || h.includes("777f")) return "pmdg-boeing-777f";
      if (h.includes("77l") || h.includes("200lr")) return "pmdg-777-200lr"; // ¡sin "boeing"!
      if (h.includes("772") || h.includes("200er")) return "pmdg-boeing-777-200er";
      if (h.includes("77w") || h.includes("300er") || h.includes("777"))
        return "pmdg-boeing-777-300er";
      // Familia 737
      if (h.includes("739") || h.includes("737-900") || h.includes("737900"))
        return "pmdg-boeing-737-900";
      if (h.includes("737-700") || h.includes("737700"))
        return "pmdg-boeing-737-700";
      if (h.includes("736") || h.includes("737-600"))
        return "pmdg-boeing-737-600";
      if (h.includes("738") || h.includes("737")) return "pmdg-boeing-737-800";
      return null;
    },
  },
  {
    label: "FlyByWire A32NX / A380X",
    resolve: (h) => {
      if (!h.includes("flybywire") && !/\bfbw\b/.test(h) && !h.includes("a32nx"))
        return null;
      if (h.includes("a380") || h.includes("380x")) return "flybywire-a380x";
      return "flybywire-a32nx";
    },
  },
  {
    label: "Headwind A330-900 (A339X)",
    resolve: (h) => {
      if (!h.includes("headwind")) return null;
      // La página de flightsim.to es `airbus-a330-900` (sin prefijo de marca).
      return "airbus-a330-900";
    },
  },
  {
    label: "Salty 747-8",
    resolve: (h) => {
      if (h.includes("salty") && (h.includes("747") || h.includes("74"))) {
        return "salty-simulations-b747-8";
      }
      return null;
    },
  },
  {
    label: "TFDi Design MD-11",
    resolve: (h) =>
      h.includes("tfdi") || h.includes("md-11") || h.includes("md11")
        ? "tfdi-design-md-11"
        : null,
  },
  {
    label: "Aerosoft CRJ (550 · 700 · 900/1000)",
    resolve: (h) => {
      if (!h.includes("aerosoft") || !h.includes("crj")) return null;
      // OJO: el slug combinado 550-700 NO existe (404); están separados.
      if (h.includes("550")) return "aerosoft-crj-550";
      if (h.includes("700")) return "aerosoft-crj-700";
      return "aerosoft-crj-900-1000";
    },
  },
  {
    label: "Leonardo / Fly the Maddog X (MD-80)",
    resolve: (h) => {
      if (h.includes("maddog")) return "fly-the-maddog-x";
      if (h.includes("leonardo") && /md[-_ ]?8\d/.test(h))
        return "fly-the-maddog-x";
      return null;
    },
  },
  {
    label: "Black Square (Caravan · TBM-850)",
    resolve: (h) => {
      if (!h.includes("black square") && !h.includes("blacksquare")) return null;
      if (h.includes("caravan")) return "black-square-caravan-professional";
      if (h.includes("tbm")) return "black-square-tbm-850";
      return null;
    },
  },
  {
    label: "Just Flight Fokker F100 Professional",
    resolve: (h) => {
      if (!h.includes("just flight") && !h.includes("justflight")) return null;
      if (h.includes("f100") || h.includes("fokker"))
        return "just-flight-fokker-f100-professional";
      return null;
    },
  },
];

/** Heno de búsqueda: creator + título + carpeta, en minúsculas. */
function haystack(pkg: CommunityPackage): string {
  return `${pkg.creator ?? ""} ${pkg.title ?? ""} ${pkg.folderName ?? ""}`
    .toLowerCase()
    .trim();
}

/** Slug de flightsim.to o `null` si no hay página dedicada conocida. */
export function liverySlugFor(pkg: CommunityPackage): string | null {
  const h = haystack(pkg);
  for (const r of RESOLVERS) {
    const slug = r.resolve(h);
    if (slug) return slug;
  }
  return null;
}

/**
 * Término de búsqueda LIMPIO y CORTO para el fallback. La búsqueda de
 * flightsim.to es AND por tokens, así que:
 *  · quitamos ruido: paréntesis "(branch HEAD)"/"(Early Access)", versiones,
 *    marcadores de build;
 *  · recortamos el sufijo de variante tras un guion ("747-8"→"747",
 *    "A330-900"→"A330") que dispara 0 resultados;
 *  · componemos marca (1ª palabra del creator) + modelo, sin duplicar, máx 3
 *    tokens. Así "Headwind Simulations / A339X (branch HEAD)" → "Headwind A339X",
 *    y un avión nuevo cualquiera cae en "<marca> <modelo>" con resultados reales.
 */
export function liverySearchTermFor(pkg: CommunityPackage): string {
  const creator = (pkg.creator ?? "").trim();
  const brand = (creator.split(/[\s|/,]+/).filter(Boolean)[0] ?? "").trim();
  const cleanedTitle = (pkg.title ?? "")
    .replace(/\([^)]*\)/g, " ") // (branch HEAD), (Early Access), (PMDG772)
    .replace(/\b(branch\s+\S+|early\s+access|development|beta|wip|rev\d*)\b/gi, " ")
    .replace(/\bv?\d+\.\d[\w.-]*/gi, " ") // v1.2.3 / 2024.2.0-hash
    .replace(/\s+/g, " ")
    .trim();

  const out: string[] = [];
  const seen = new Set<string>();
  for (const raw of `${brand} ${cleanedTitle}`.split(/\s+/)) {
    // Recorta el sufijo de variante tras un guion: "747-8" → "747".
    const tok = raw.split(/[-–—]/)[0];
    const key = tok.toLowerCase().replace(/[^a-z0-9]/g, "");
    if (!key || seen.has(key)) continue;
    seen.add(key);
    out.push(tok);
    if (out.length >= 3) break; // corto (la búsqueda es AND)
  }
  return (out.join(" ") || cleanedTitle || brand || (pkg.title ?? "")).trim();
}

/**
 * URL de flightsim.to para ver las liveries de este avión. SIEMPRE devuelve
 * algo útil: la página dedicada si la conocemos, o la búsqueda real acotada a
 * Liveries para cualquier otro avión (incluidos los que agregues después).
 */
export function liveryUrlFor(pkg: CommunityPackage): string {
  const slug = liverySlugFor(pkg);
  if (slug) return `${LIVERIES}/${slug}`;
  const term = liverySearchTermFor(pkg);
  return `${BASE}/search?q=${encodeURIComponent(term)}&cat=liveries`;
}
