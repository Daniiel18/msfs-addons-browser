/**
 * (v7.5) Clasificación de ORIGEN de un addon para la re-descarga selectiva
 * al importar un inventario. Dado el `creator`/título de un addon, decide
 * dónde conviene buscarlo para re-bajarlo:
 *
 *   · "catalog"     — está en una fuente in-app (SceneryAddons/Simplaza/
 *                     Skybound); se baja por la app.
 *   · "flightsimto" — freeware; abrimos flightsim.to (webview o búsqueda).
 *   · "payware"     — es de un desarrollador de pago; NO se puede bajar
 *                     automático: abrimos su página/tienda OFICIAL.
 *   · "skip"        — el usuario decide no bajarlo.
 *
 * El mapa de payware está modelado como `liveryslugs.ts` (substring del
 * creator → URL oficial). NO pretende ser exhaustivo — cubre los devs de
 * pago más comunes de MSFS; el resto cae a una búsqueda en flightsim.to.
 */

export type OriginKind = "catalog" | "flightsimto" | "payware" | "skip";

export interface PaywareVendor {
  /** Nombre visible del desarrollador/tienda. */
  name: string;
  /** Página oficial (o de tienda) para conseguir el addon. */
  url: string;
}

interface VendorRule {
  /** Substrings (en minúscula) del creator/manufacturer del manifest. */
  match: string[];
  vendor: PaywareVendor;
}

/**
 * Desarrolladores de PAGO (payware). Si el `creator` del addon matchea, no
 * se puede bajar automático: se abre su sitio oficial. Los substrings se
 * comparan en minúscula contra creator + título.
 */
const PAYWARE_VENDORS: VendorRule[] = [
  { match: ["pmdg"], vendor: { name: "PMDG", url: "https://pmdg.com/" } },
  { match: ["fenix"], vendor: { name: "Fenix Simulations", url: "https://fenixsim.com/" } },
  { match: ["inibuilds", "ini builds", "iniscene", "inibuild"], vendor: { name: "iniBuilds", url: "https://www.inibuilds.com/" } },
  { match: ["aerosoft"], vendor: { name: "Aerosoft", url: "https://www.aerosoft.com/en/" } },
  { match: ["leonardo", "maddog"], vendor: { name: "Leonardo (Maddog)", url: "https://www.maddog-x.com/" } },
  { match: ["captain sim", "captainsim"], vendor: { name: "Captain Sim", url: "https://captainsim.com/" } },
  { match: ["just flight", "justflight", "black square", "blacksquare"], vendor: { name: "Just Flight", url: "https://www.justflight.com/" } },
  { match: ["carenado"], vendor: { name: "Carenado", url: "https://www.carenado.com/" } },
  { match: ["milviz", "blackbird"], vendor: { name: "Blackbird (MilViz)", url: "https://www.blackbirdsim.com/" } },
  { match: ["tfdi", "tfdidesign"], vendor: { name: "TFDi Design", url: "https://www.tfdidesign.com/" } },
  { match: ["ifly"], vendor: { name: "iFly Simulations", url: "https://www.iflysimsoft.com/" } },
  { match: ["flightfactor", "flight factor"], vendor: { name: "FlightFactor", url: "https://www.flightfactor.aero/" } },
  { match: ["latinvfr", "lvfr", "latin vfr"], vendor: { name: "LatinVFR", url: "https://www.latinvfr.com/" } },
  { match: ["flytampa"], vendor: { name: "FlyTampa", url: "https://www.flytampa.org/" } },
  { match: ["fsdreamteam", "fsdt", "gsx"], vendor: { name: "FSDreamTeam", url: "https://www.fsdreamteam.com/" } },
  { match: ["orbx"], vendor: { name: "Orbx", url: "https://orbxdirect.com/" } },
  { match: ["flightbeam"], vendor: { name: "Flightbeam", url: "https://www.flightbeamstore.com/" } },
  { match: ["fsimstudios", "fsim studios"], vendor: { name: "FSimStudios", url: "https://fsimstudios.com/" } },
  { match: ["bredok3d", "bredok"], vendor: { name: "Bredok3D", url: "https://bredok3d.com/" } },
  { match: ["flightsim studio", "fss "], vendor: { name: "FlightSim Studio", url: "https://flightsimstudioag.com/" } },
];

/**
 * Marcas conocidas de FREEWARE distribuido en flightsim.to (aunque suenen
 * a "dev de pago"). Fuerza `flightsimto` en vez de payware.
 */
const FREEWARE_ONLINE = [
  "flybywire",
  "fly by wire",
  "fbw",
  "salty",
  "headwind",
  "working title",
  "workingtitle",
  "synaptic",
  "horizonsim",
  "horizon sim",
];

/** Normaliza a minúscula + colapsa espacios para comparar. */
function norm(s: string | null | undefined): string {
  return (s ?? "").toLowerCase().replace(/\s+/g, " ").trim();
}

/** ¿El creator/título es freeware conocido de flightsim.to? */
export function isFreewareOnline(creator?: string | null, title?: string | null): boolean {
  const hay = `${norm(creator)} ${norm(title)}`;
  return FREEWARE_ONLINE.some((f) => hay.includes(f));
}

/**
 * Devuelve el vendor de pago si el creator/título matchea un dev payware
 * conocido (y NO es freeware-online). `null` si no es payware conocido.
 */
export function paywareVendorFor(
  creator?: string | null,
  title?: string | null,
): PaywareVendor | null {
  if (isFreewareOnline(creator, title)) return null;
  const hay = `${norm(creator)} ${norm(title)}`;
  for (const rule of PAYWARE_VENDORS) {
    if (rule.match.some((m) => hay.includes(m))) return rule.vendor;
  }
  return null;
}

/** URL de búsqueda en flightsim.to para un addon por título (+creator). */
export function flightsimSearchUrl(title: string, creator?: string | null): string {
  // Términos cortos y significativos: quitamos palabras de ruido para que
  // la búsqueda AND de flightsim.to no devuelva vacío.
  const stop = new Set([
    "airport", "aeropuerto", "scenery", "aircraft", "for", "msfs", "2020",
    "2024", "the", "and", "de", "la", "el", "international",
  ]);
  const base = `${title} ${creator ?? ""}`
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, " ")
    .split(/\s+/)
    .filter((w) => w.length > 1 && !stop.has(w))
    .slice(0, 5)
    .join(" ")
    .trim();
  const q = encodeURIComponent(base || title);
  return `https://flightsim.to/search?q=${q}`;
}
