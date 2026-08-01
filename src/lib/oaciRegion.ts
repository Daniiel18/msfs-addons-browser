/**
 * (v5.0.0) Identificadores regionales a partir del código ICAO.
 *
 * El primer carácter del ICAO fija el "continente/zona" OACI; los dos
 * primeros suelen fijar el PAÍS (LE=España, MD=Rep. Dominicana,
 * KMIA=EE.UU., …). Con eso pintamos un micro-badge de color en cada
 * card de aeropuerto (Map, card flotante, Link Map) y agrupamos los
 * aeropuertos por región en el lienzo del Link Map.
 *
 * Compartido por LinkMapView, MapView, MapAirportCard y el dashboard
 * para que la clasificación geográfica sea una sola fuente de verdad.
 *
 * Las etiquetas legibles se guardan como CLAVES i18n (`labelKey`) y se
 * resuelven con `t()` en el momento de leerlas — así el badge respeta
 * el idioma activo sin duplicar la tabla por locale.
 */

import { t } from "./i18n";

export type Continent =
  | "north_america"
  | "central_america"
  | "caribbean"
  | "south_america"
  | "europe"
  | "africa"
  | "middle_east"
  | "asia"
  | "oceania"
  | "other";

export interface AirportRegion {
  /** Código corto para el badge: país (2 letras) si lo conocemos, si no
   *  la inicial OACI. Ej. "ES", "DO", "US", "L". */
  code: string;
  /** Nombre legible del país/zona ya resuelto al idioma activo.
   *  Ej. "España", "Rep. Dominicana". */
  label: string;
  /** Continente/macro-zona para agrupar y colorear. */
  continent: Continent;
}

// País por prefijo OACI de 2 letras. Cubre con detalle los destinos más
// comunes (América + Europa) y deja el resto al fallback de 1 letra.
// No pretende ser exhaustivo: lo que no esté aquí cae al continente.
// `labelKey` es una clave i18n resuelta con t() al leer (ver airportRegion).
const COUNTRY_2: Record<string, { code: string; labelKey: string; continent: Continent }> = {
  // Norteamérica
  KA: { code: "USA", labelKey: "region.country.KA", continent: "north_america" },
  // (las K* son todas EE.UU.; se resuelven por inicial abajo)
  CY: { code: "CA", labelKey: "region.country.CY", continent: "north_america" },
  CZ: { code: "CA", labelKey: "region.country.CZ", continent: "north_america" },
  PA: { code: "USA", labelKey: "region.country.PA", continent: "north_america" },
  PH: { code: "USA", labelKey: "region.country.PH", continent: "north_america" },
  PG: { code: "GU", labelKey: "region.country.PG", continent: "oceania" },
  // México y Centroamérica
  MM: { code: "MX", labelKey: "region.country.MM", continent: "central_america" },
  MG: { code: "GT", labelKey: "region.country.MG", continent: "central_america" },
  MS: { code: "SV", labelKey: "region.country.MS", continent: "central_america" },
  MH: { code: "HN", labelKey: "region.country.MH", continent: "central_america" },
  MN: { code: "NI", labelKey: "region.country.MN", continent: "central_america" },
  MR: { code: "CR", labelKey: "region.country.MR", continent: "central_america" },
  MP: { code: "PA", labelKey: "region.country.MP", continent: "central_america" },
  MZ: { code: "BZ", labelKey: "region.country.MZ", continent: "central_america" },
  // Caribe
  MD: { code: "DO", labelKey: "region.country.MD", continent: "caribbean" },
  MT: { code: "HT", labelKey: "region.country.MT", continent: "caribbean" },
  MU: { code: "CU", labelKey: "region.country.MU", continent: "caribbean" },
  MK: { code: "JM", labelKey: "region.country.MK", continent: "caribbean" },
  MY: { code: "BS", labelKey: "region.country.MY", continent: "caribbean" },
  MB: { code: "TC", labelKey: "region.country.MB", continent: "caribbean" },
  MW: { code: "KY", labelKey: "region.country.MW", continent: "caribbean" },
  TJ: { code: "PR", labelKey: "region.country.TJ", continent: "caribbean" },
  TI: { code: "VI", labelKey: "region.country.TI", continent: "caribbean" },
  TN: { code: "AN", labelKey: "region.country.TN", continent: "caribbean" },
  TT: { code: "TT", labelKey: "region.country.TT", continent: "caribbean" },
  TB: { code: "BB", labelKey: "region.country.TB", continent: "caribbean" },
  TF: { code: "FR", labelKey: "region.country.TF", continent: "caribbean" },
  TA: { code: "AG", labelKey: "region.country.TA", continent: "caribbean" },
  TL: { code: "LC", labelKey: "region.country.TL", continent: "caribbean" },
  TG: { code: "GD", labelKey: "region.country.TG", continent: "caribbean" },
  TK: { code: "KN", labelKey: "region.country.TK", continent: "caribbean" },
  TV: { code: "VC", labelKey: "region.country.TV", continent: "caribbean" },
  TD: { code: "DM", labelKey: "region.country.TD", continent: "caribbean" },
  // Sudamérica
  SA: { code: "AR", labelKey: "region.country.SA", continent: "south_america" },
  SC: { code: "CL", labelKey: "region.country.SC", continent: "south_america" },
  SE: { code: "EC", labelKey: "region.country.SE", continent: "south_america" },
  SG: { code: "PY", labelKey: "region.country.SG", continent: "south_america" },
  SK: { code: "CO", labelKey: "region.country.SK", continent: "south_america" },
  SL: { code: "BO", labelKey: "region.country.SL", continent: "south_america" },
  SM: { code: "SR", labelKey: "region.country.SM", continent: "south_america" },
  SO: { code: "GF", labelKey: "region.country.SO", continent: "south_america" },
  SP: { code: "PE", labelKey: "region.country.SP", continent: "south_america" },
  SU: { code: "UY", labelKey: "region.country.SU", continent: "south_america" },
  SV: { code: "VE", labelKey: "region.country.SV", continent: "south_america" },
  SY: { code: "GY", labelKey: "region.country.SY", continent: "south_america" },
  SB: { code: "BR", labelKey: "region.country.SB", continent: "south_america" },
  SD: { code: "BR", labelKey: "region.country.SB", continent: "south_america" },
  SI: { code: "BR", labelKey: "region.country.SB", continent: "south_america" },
  SJ: { code: "BR", labelKey: "region.country.SB", continent: "south_america" },
  SN: { code: "BR", labelKey: "region.country.SB", continent: "south_america" },
  SS: { code: "BR", labelKey: "region.country.SB", continent: "south_america" },
  SW: { code: "BR", labelKey: "region.country.SB", continent: "south_america" },
  // Europa
  LE: { code: "ES", labelKey: "region.country.LE", continent: "europe" },
  GC: { code: "ES", labelKey: "region.country.GC", continent: "europe" },
  LF: { code: "FR", labelKey: "region.country.LF", continent: "europe" },
  LI: { code: "IT", labelKey: "region.country.LI", continent: "europe" },
  LP: { code: "PT", labelKey: "region.country.LP", continent: "europe" },
  LG: { code: "GR", labelKey: "region.country.LG", continent: "europe" },
  LH: { code: "HU", labelKey: "region.country.LH", continent: "europe" },
  LO: { code: "AT", labelKey: "region.country.LO", continent: "europe" },
  LS: { code: "CH", labelKey: "region.country.LS", continent: "europe" },
  LK: { code: "CZ", labelKey: "region.country.LK", continent: "europe" },
  LZ: { code: "SK", labelKey: "region.country.LZ", continent: "europe" },
  LR: { code: "RO", labelKey: "region.country.LR", continent: "europe" },
  LB: { code: "BG", labelKey: "region.country.LB", continent: "europe" },
  LT: { code: "TR", labelKey: "region.country.LT", continent: "europe" },
  LD: { code: "HR", labelKey: "region.country.LD", continent: "europe" },
  LJ: { code: "SI", labelKey: "region.country.LJ", continent: "europe" },
  LY: { code: "RS", labelKey: "region.country.LY", continent: "europe" },
  LM: { code: "MT", labelKey: "region.country.LM", continent: "europe" },
  LC: { code: "CY", labelKey: "region.country.LC", continent: "middle_east" },
  LL: { code: "IL", labelKey: "region.country.LL", continent: "middle_east" },
  EG: { code: "GB", labelKey: "region.country.EG", continent: "europe" },
  EH: { code: "NL", labelKey: "region.country.EH", continent: "europe" },
  EB: { code: "BE", labelKey: "region.country.EB", continent: "europe" },
  ED: { code: "DE", labelKey: "region.country.ED", continent: "europe" },
  ET: { code: "DE", labelKey: "region.country.ED", continent: "europe" },
  EK: { code: "DK", labelKey: "region.country.EK", continent: "europe" },
  EN: { code: "NO", labelKey: "region.country.EN", continent: "europe" },
  ES: { code: "SE", labelKey: "region.country.ES", continent: "europe" },
  EF: { code: "FI", labelKey: "region.country.EF", continent: "europe" },
  EI: { code: "IE", labelKey: "region.country.EI", continent: "europe" },
  EP: { code: "PL", labelKey: "region.country.EP", continent: "europe" },
  EV: { code: "LV", labelKey: "region.country.EV", continent: "europe" },
  EY: { code: "LT", labelKey: "region.country.EY", continent: "europe" },
  EE: { code: "EE", labelKey: "region.country.EE", continent: "europe" },
  BI: { code: "IS", labelKey: "region.country.BI", continent: "europe" },
  BG: { code: "GL", labelKey: "region.country.BG", continent: "north_america" },
  // Rusia / CIS
  UU: { code: "RU", labelKey: "region.country.UU", continent: "europe" },
  UL: { code: "RU", labelKey: "region.country.UU", continent: "europe" },
  UW: { code: "RU", labelKey: "region.country.UU", continent: "europe" },
  UK: { code: "UA", labelKey: "region.country.UK", continent: "europe" },
  UB: { code: "AZ", labelKey: "region.country.UB", continent: "asia" },
  UD: { code: "AM", labelKey: "region.country.UD", continent: "asia" },
  UG: { code: "GE", labelKey: "region.country.UG", continent: "asia" },
  UT: { code: "UZ", labelKey: "region.country.UT", continent: "asia" },
  // Oriente Medio
  OM: { code: "AE", labelKey: "region.country.OM", continent: "middle_east" },
  OT: { code: "QA", labelKey: "region.country.OT", continent: "middle_east" },
  OB: { code: "BH", labelKey: "region.country.OB", continent: "middle_east" },
  OK: { code: "KW", labelKey: "region.country.OK", continent: "middle_east" },
  OE: { code: "SA", labelKey: "region.country.OE", continent: "middle_east" },
  OO: { code: "OM", labelKey: "region.country.OO", continent: "middle_east" },
  OI: { code: "IR", labelKey: "region.country.OI", continent: "middle_east" },
  OJ: { code: "JO", labelKey: "region.country.OJ", continent: "middle_east" },
  OL: { code: "LB", labelKey: "region.country.OL", continent: "middle_east" },
  OR: { code: "IQ", labelKey: "region.country.OR", continent: "middle_east" },
  OP: { code: "PK", labelKey: "region.country.OP", continent: "asia" },
  // Asia
  RJ: { code: "JP", labelKey: "region.country.RJ", continent: "asia" },
  RO: { code: "JP", labelKey: "region.country.RO", continent: "asia" },
  RK: { code: "KR", labelKey: "region.country.RK", continent: "asia" },
  RC: { code: "TW", labelKey: "region.country.RC", continent: "asia" },
  RP: { code: "PH", labelKey: "region.country.RP", continent: "asia" },
  VT: { code: "TH", labelKey: "region.country.VT", continent: "asia" },
  VV: { code: "VN", labelKey: "region.country.VV", continent: "asia" },
  VH: { code: "HK", labelKey: "region.country.VH", continent: "asia" },
  VM: { code: "MO", labelKey: "region.country.VM", continent: "asia" },
  VD: { code: "KH", labelKey: "region.country.VD", continent: "asia" },
  VL: { code: "LA", labelKey: "region.country.VL", continent: "asia" },
  VY: { code: "MM", labelKey: "region.country.VY", continent: "asia" },
  VN: { code: "NP", labelKey: "region.country.VN", continent: "asia" },
  VA: { code: "IN", labelKey: "region.country.VA", continent: "asia" },
  VE: { code: "IN", labelKey: "region.country.VA", continent: "asia" },
  VI: { code: "IN", labelKey: "region.country.VA", continent: "asia" },
  VO: { code: "IN", labelKey: "region.country.VA", continent: "asia" },
  VC: { code: "LK", labelKey: "region.country.VC", continent: "asia" },
  VG: { code: "BD", labelKey: "region.country.VG", continent: "asia" },
  VR: { code: "MV", labelKey: "region.country.VR", continent: "asia" },
  WI: { code: "ID", labelKey: "region.country.WI", continent: "asia" },
  WA: { code: "ID", labelKey: "region.country.WI", continent: "asia" },
  WR: { code: "ID", labelKey: "region.country.WI", continent: "asia" },
  WM: { code: "MY", labelKey: "region.country.WM", continent: "asia" },
  WB: { code: "MY", labelKey: "region.country.WB", continent: "asia" },
  WS: { code: "SG", labelKey: "region.country.WS", continent: "asia" },
  ZK: { code: "KP", labelKey: "region.country.ZK", continent: "asia" },
  ZM: { code: "MN", labelKey: "region.country.ZM", continent: "asia" },
  // África
  FA: { code: "ZA", labelKey: "region.country.FA", continent: "africa" },
  FM: { code: "MG", labelKey: "region.country.FM", continent: "africa" },
  FV: { code: "ZW", labelKey: "region.country.FV", continent: "africa" },
  FL: { code: "ZM", labelKey: "region.country.FL", continent: "africa" },
  FN: { code: "AO", labelKey: "region.country.FN", continent: "africa" },
  FQ: { code: "MZ", labelKey: "region.country.FQ", continent: "africa" },
  HE: { code: "EG", labelKey: "region.country.HE", continent: "africa" },
  HK: { code: "KE", labelKey: "region.country.HK", continent: "africa" },
  HT: { code: "TZ", labelKey: "region.country.HT", continent: "africa" },
  HU: { code: "UG", labelKey: "region.country.HU", continent: "africa" },
  HH: { code: "ER", labelKey: "region.country.HH", continent: "africa" },
  HA: { code: "ET", labelKey: "region.country.HA", continent: "africa" },
  HL: { code: "LY", labelKey: "region.country.HL", continent: "africa" },
  HS: { code: "SD", labelKey: "region.country.HS", continent: "africa" },
  GM: { code: "MA", labelKey: "region.country.GM", continent: "africa" },
  DT: { code: "TN", labelKey: "region.country.DT", continent: "africa" },
  DA: { code: "DZ", labelKey: "region.country.DA", continent: "africa" },
  DN: { code: "NG", labelKey: "region.country.DN", continent: "africa" },
  DG: { code: "GH", labelKey: "region.country.DG", continent: "africa" },
  DI: { code: "CI", labelKey: "region.country.DI", continent: "africa" },
  GO: { code: "SN", labelKey: "region.country.GO", continent: "africa" },
  GV: { code: "CV", labelKey: "region.country.GV", continent: "africa" },
  // Oceanía
  YM: { code: "AU", labelKey: "region.country.YM", continent: "oceania" },
  YS: { code: "AU", labelKey: "region.country.YM", continent: "oceania" },
  YB: { code: "AU", labelKey: "region.country.YM", continent: "oceania" },
  YP: { code: "AU", labelKey: "region.country.YM", continent: "oceania" },
  YW: { code: "AU", labelKey: "region.country.YM", continent: "oceania" },
  YL: { code: "AU", labelKey: "region.country.YM", continent: "oceania" },
  YC: { code: "AU", labelKey: "region.country.YM", continent: "oceania" },
  NZ: { code: "NZ", labelKey: "region.country.NZ", continent: "oceania" },
  NF: { code: "FJ", labelKey: "region.country.NF", continent: "oceania" },
  NT: { code: "PF", labelKey: "region.country.NT", continent: "oceania" },
  NW: { code: "NC", labelKey: "region.country.NW", continent: "oceania" },
  AY: { code: "PG", labelKey: "region.country.AY", continent: "oceania" },
};

// Fallback por inicial OACI cuando el país de 2 letras no está en la
// tabla. Incluye un `code` corto y legible (p. ej. K* y P* → "USA").
// `labelKey` es una clave i18n resuelta con t() al leer (ver airportRegion).
const REGION_1: Record<string, { code: string; labelKey: string; continent: Continent }> = {
  K: { code: "USA", labelKey: "region.fallback.K", continent: "north_america" },
  C: { code: "CAN", labelKey: "region.fallback.C", continent: "north_america" },
  P: { code: "USA", labelKey: "region.fallback.P", continent: "north_america" },
  M: { code: "C.AM", labelKey: "region.fallback.M", continent: "central_america" },
  T: { code: "CARIB", labelKey: "region.fallback.T", continent: "caribbean" },
  S: { code: "S.AM", labelKey: "region.fallback.S", continent: "south_america" },
  E: { code: "EUR", labelKey: "region.fallback.E", continent: "europe" },
  L: { code: "EUR", labelKey: "region.fallback.L", continent: "europe" },
  B: { code: "EUR", labelKey: "region.fallback.B", continent: "europe" },
  U: { code: "RU", labelKey: "region.fallback.U", continent: "europe" },
  R: { code: "ASIA", labelKey: "region.fallback.R", continent: "asia" },
  Z: { code: "CN", labelKey: "region.fallback.Z", continent: "asia" },
  V: { code: "ASIA", labelKey: "region.fallback.V", continent: "asia" },
  W: { code: "ASIA", labelKey: "region.fallback.W", continent: "asia" },
  O: { code: "M.E", labelKey: "region.fallback.O", continent: "middle_east" },
  F: { code: "AFR", labelKey: "region.fallback.F", continent: "africa" },
  G: { code: "AFR", labelKey: "region.fallback.G", continent: "africa" },
  H: { code: "AFR", labelKey: "region.fallback.H", continent: "africa" },
  D: { code: "AFR", labelKey: "region.fallback.D", continent: "africa" },
  A: { code: "OC", labelKey: "region.fallback.A", continent: "oceania" },
  N: { code: "OC", labelKey: "region.fallback.N", continent: "oceania" },
  Y: { code: "AUS", labelKey: "region.fallback.Y", continent: "oceania" },
};

// Clave i18n de la etiqueta legible por continente — resuelta con t()
// en continentLabel() para respetar el idioma activo.
const CONTINENT_LABEL_KEY: Record<Continent, string> = {
  north_america: "region.continent.north_america",
  central_america: "region.continent.central_america",
  caribbean: "region.continent.caribbean",
  south_america: "region.continent.south_america",
  europe: "region.continent.europe",
  africa: "region.continent.africa",
  middle_east: "region.continent.middle_east",
  asia: "region.continent.asia",
  oceania: "region.continent.oceania",
  other: "region.continent.other",
};

/** Clases Tailwind (texto + fondo + ring) por continente para los badges.
 *  (v5.0.0) Más contraste: texto -100, fondo /30, ring /60. */
const CONTINENT_BADGE: Record<Continent, string> = {
  north_america: "bg-sky-500/30 text-sky-100 ring-sky-400/60",
  central_america: "bg-orange-500/30 text-orange-100 ring-orange-400/60",
  caribbean: "bg-teal-500/30 text-teal-100 ring-teal-400/60",
  south_america: "bg-amber-500/30 text-amber-100 ring-amber-400/60",
  europe: "bg-indigo-500/30 text-indigo-100 ring-indigo-400/60",
  africa: "bg-lime-500/30 text-lime-100 ring-lime-400/60",
  middle_east: "bg-rose-500/30 text-rose-100 ring-rose-400/60",
  asia: "bg-fuchsia-500/30 text-fuchsia-100 ring-fuchsia-400/60",
  oceania: "bg-emerald-500/30 text-emerald-100 ring-emerald-400/60",
  other: "bg-slate-600/40 text-slate-100 ring-slate-500/60",
};

/** Resuelve el país/zona de un ICAO. Nunca lanza; "Otros" si no hay ICAO. */
export function airportRegion(icao: string | null | undefined): AirportRegion {
  const c = (icao ?? "").trim().toUpperCase();
  if (c.length < 1) {
    return { code: "?", label: t("region.continent.other"), continent: "other" };
  }
  const two = c.slice(0, 2);
  const hit2 = COUNTRY_2[two];
  if (hit2) {
    return { code: hit2.code, label: t(hit2.labelKey), continent: hit2.continent };
  }
  const hit1 = REGION_1[c[0]];
  if (hit1) {
    return { code: hit1.code, label: t(hit1.labelKey), continent: hit1.continent };
  }
  return { code: c[0], label: t("region.continent.other"), continent: "other" };
}

/** Etiqueta legible de la macro-zona (continente) — para agrupar. */
export function continentLabel(cont: Continent): string {
  return t(CONTINENT_LABEL_KEY[cont]);
}

/** Clases del micro-badge (color por continente). */
export function regionBadgeClass(cont: Continent): string {
  return CONTINENT_BADGE[cont];
}

/** (compat) Etiqueta de zona estilo v4.33 — usada para ordenar/agrupar
 *  los aeropuertos en el Link Map por su continente. */
export function oaciRegionLabel(icao: string | null | undefined): string {
  return continentLabel(airportRegion(icao).continent);
}
