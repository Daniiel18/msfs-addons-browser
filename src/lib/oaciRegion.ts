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
 */

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
  /** Nombre legible del país/zona. Ej. "España", "Rep. Dominicana". */
  label: string;
  /** Continente/macro-zona para agrupar y colorear. */
  continent: Continent;
}

// País por prefijo OACI de 2 letras. Cubre con detalle los destinos más
// comunes (América + Europa) y deja el resto al fallback de 1 letra.
// No pretende ser exhaustivo: lo que no esté aquí cae al continente.
const COUNTRY_2: Record<string, { code: string; label: string; continent: Continent }> = {
  // Norteamérica
  KA: { code: "USA", label: "EE.UU.", continent: "north_america" },
  // (las K* son todas EE.UU.; se resuelven por inicial abajo)
  CY: { code: "CA", label: "Canadá", continent: "north_america" },
  CZ: { code: "CA", label: "Canadá", continent: "north_america" },
  PA: { code: "USA", label: "EE.UU. (Alaska)", continent: "north_america" },
  PH: { code: "USA", label: "EE.UU. (Hawái)", continent: "north_america" },
  PG: { code: "GU", label: "Guam", continent: "oceania" },
  // México y Centroamérica
  MM: { code: "MX", label: "México", continent: "central_america" },
  MG: { code: "GT", label: "Guatemala", continent: "central_america" },
  MS: { code: "SV", label: "El Salvador", continent: "central_america" },
  MH: { code: "HN", label: "Honduras", continent: "central_america" },
  MN: { code: "NI", label: "Nicaragua", continent: "central_america" },
  MR: { code: "CR", label: "Costa Rica", continent: "central_america" },
  MP: { code: "PA", label: "Panamá", continent: "central_america" },
  MZ: { code: "BZ", label: "Belice", continent: "central_america" },
  // Caribe
  MD: { code: "DO", label: "Rep. Dominicana", continent: "caribbean" },
  MT: { code: "HT", label: "Haití", continent: "caribbean" },
  MU: { code: "CU", label: "Cuba", continent: "caribbean" },
  MK: { code: "JM", label: "Jamaica", continent: "caribbean" },
  MY: { code: "BS", label: "Bahamas", continent: "caribbean" },
  MB: { code: "TC", label: "Turcas y Caicos", continent: "caribbean" },
  MW: { code: "KY", label: "Caimán", continent: "caribbean" },
  TJ: { code: "PR", label: "Puerto Rico", continent: "caribbean" },
  TI: { code: "VI", label: "Islas Vírgenes (EE.UU.)", continent: "caribbean" },
  TN: { code: "AN", label: "Antillas (NL)", continent: "caribbean" },
  TT: { code: "TT", label: "Trinidad y Tobago", continent: "caribbean" },
  TB: { code: "BB", label: "Barbados", continent: "caribbean" },
  TF: { code: "FR", label: "Antillas (FR)", continent: "caribbean" },
  TA: { code: "AG", label: "Antigua y Barbuda", continent: "caribbean" },
  TL: { code: "LC", label: "Santa Lucía", continent: "caribbean" },
  TG: { code: "GD", label: "Granada", continent: "caribbean" },
  TK: { code: "KN", label: "San Cristóbal", continent: "caribbean" },
  TV: { code: "VC", label: "San Vicente", continent: "caribbean" },
  TD: { code: "DM", label: "Dominica", continent: "caribbean" },
  // Sudamérica
  SA: { code: "AR", label: "Argentina", continent: "south_america" },
  SC: { code: "CL", label: "Chile", continent: "south_america" },
  SE: { code: "EC", label: "Ecuador", continent: "south_america" },
  SG: { code: "PY", label: "Paraguay", continent: "south_america" },
  SK: { code: "CO", label: "Colombia", continent: "south_america" },
  SL: { code: "BO", label: "Bolivia", continent: "south_america" },
  SM: { code: "SR", label: "Surinam", continent: "south_america" },
  SO: { code: "GF", label: "Guayana Francesa", continent: "south_america" },
  SP: { code: "PE", label: "Perú", continent: "south_america" },
  SU: { code: "UY", label: "Uruguay", continent: "south_america" },
  SV: { code: "VE", label: "Venezuela", continent: "south_america" },
  SY: { code: "GY", label: "Guyana", continent: "south_america" },
  SB: { code: "BR", label: "Brasil", continent: "south_america" },
  SD: { code: "BR", label: "Brasil", continent: "south_america" },
  SI: { code: "BR", label: "Brasil", continent: "south_america" },
  SJ: { code: "BR", label: "Brasil", continent: "south_america" },
  SN: { code: "BR", label: "Brasil", continent: "south_america" },
  SS: { code: "BR", label: "Brasil", continent: "south_america" },
  SW: { code: "BR", label: "Brasil", continent: "south_america" },
  // Europa
  LE: { code: "ES", label: "España", continent: "europe" },
  GC: { code: "ES", label: "España (Canarias)", continent: "europe" },
  LF: { code: "FR", label: "Francia", continent: "europe" },
  LI: { code: "IT", label: "Italia", continent: "europe" },
  LP: { code: "PT", label: "Portugal", continent: "europe" },
  LG: { code: "GR", label: "Grecia", continent: "europe" },
  LH: { code: "HU", label: "Hungría", continent: "europe" },
  LO: { code: "AT", label: "Austria", continent: "europe" },
  LS: { code: "CH", label: "Suiza", continent: "europe" },
  LK: { code: "CZ", label: "Chequia", continent: "europe" },
  LZ: { code: "SK", label: "Eslovaquia", continent: "europe" },
  LR: { code: "RO", label: "Rumanía", continent: "europe" },
  LB: { code: "BG", label: "Bulgaria", continent: "europe" },
  LT: { code: "TR", label: "Turquía", continent: "europe" },
  LD: { code: "HR", label: "Croacia", continent: "europe" },
  LJ: { code: "SI", label: "Eslovenia", continent: "europe" },
  LY: { code: "RS", label: "Serbia", continent: "europe" },
  LM: { code: "MT", label: "Malta", continent: "europe" },
  LC: { code: "CY", label: "Chipre", continent: "middle_east" },
  LL: { code: "IL", label: "Israel", continent: "middle_east" },
  EG: { code: "GB", label: "Reino Unido", continent: "europe" },
  EH: { code: "NL", label: "Países Bajos", continent: "europe" },
  EB: { code: "BE", label: "Bélgica", continent: "europe" },
  ED: { code: "DE", label: "Alemania", continent: "europe" },
  ET: { code: "DE", label: "Alemania", continent: "europe" },
  EK: { code: "DK", label: "Dinamarca", continent: "europe" },
  EN: { code: "NO", label: "Noruega", continent: "europe" },
  ES: { code: "SE", label: "Suecia", continent: "europe" },
  EF: { code: "FI", label: "Finlandia", continent: "europe" },
  EI: { code: "IE", label: "Irlanda", continent: "europe" },
  EP: { code: "PL", label: "Polonia", continent: "europe" },
  EV: { code: "LV", label: "Letonia", continent: "europe" },
  EY: { code: "LT", label: "Lituania", continent: "europe" },
  EE: { code: "EE", label: "Estonia", continent: "europe" },
  BI: { code: "IS", label: "Islandia", continent: "europe" },
  BG: { code: "GL", label: "Groenlandia", continent: "north_america" },
  // Rusia / CIS
  UU: { code: "RU", label: "Rusia", continent: "europe" },
  UL: { code: "RU", label: "Rusia", continent: "europe" },
  UW: { code: "RU", label: "Rusia", continent: "europe" },
  UK: { code: "UA", label: "Ucrania", continent: "europe" },
  UB: { code: "AZ", label: "Azerbaiyán", continent: "asia" },
  UD: { code: "AM", label: "Armenia", continent: "asia" },
  UG: { code: "GE", label: "Georgia", continent: "asia" },
  UT: { code: "UZ", label: "Asia Central", continent: "asia" },
  // Oriente Medio
  OM: { code: "AE", label: "Emiratos Á.U.", continent: "middle_east" },
  OT: { code: "QA", label: "Catar", continent: "middle_east" },
  OB: { code: "BH", label: "Baréin", continent: "middle_east" },
  OK: { code: "KW", label: "Kuwait", continent: "middle_east" },
  OE: { code: "SA", label: "Arabia Saudí", continent: "middle_east" },
  OO: { code: "OM", label: "Omán", continent: "middle_east" },
  OI: { code: "IR", label: "Irán", continent: "middle_east" },
  OJ: { code: "JO", label: "Jordania", continent: "middle_east" },
  OL: { code: "LB", label: "Líbano", continent: "middle_east" },
  OR: { code: "IQ", label: "Irak", continent: "middle_east" },
  OP: { code: "PK", label: "Pakistán", continent: "asia" },
  // Asia
  RJ: { code: "JP", label: "Japón", continent: "asia" },
  RO: { code: "JP", label: "Japón (Okinawa)", continent: "asia" },
  RK: { code: "KR", label: "Corea del Sur", continent: "asia" },
  RC: { code: "TW", label: "Taiwán", continent: "asia" },
  RP: { code: "PH", label: "Filipinas", continent: "asia" },
  VT: { code: "TH", label: "Tailandia", continent: "asia" },
  VV: { code: "VN", label: "Vietnam", continent: "asia" },
  VH: { code: "HK", label: "Hong Kong", continent: "asia" },
  VM: { code: "MO", label: "Macao", continent: "asia" },
  VD: { code: "KH", label: "Camboya", continent: "asia" },
  VL: { code: "LA", label: "Laos", continent: "asia" },
  VY: { code: "MM", label: "Birmania", continent: "asia" },
  VN: { code: "NP", label: "Nepal", continent: "asia" },
  VA: { code: "IN", label: "India", continent: "asia" },
  VE: { code: "IN", label: "India", continent: "asia" },
  VI: { code: "IN", label: "India", continent: "asia" },
  VO: { code: "IN", label: "India", continent: "asia" },
  VC: { code: "LK", label: "Sri Lanka", continent: "asia" },
  VG: { code: "BD", label: "Bangladés", continent: "asia" },
  VR: { code: "MV", label: "Maldivas", continent: "asia" },
  WI: { code: "ID", label: "Indonesia", continent: "asia" },
  WA: { code: "ID", label: "Indonesia", continent: "asia" },
  WR: { code: "ID", label: "Indonesia", continent: "asia" },
  WM: { code: "MY", label: "Malasia", continent: "asia" },
  WB: { code: "MY", label: "Malasia (Borneo)", continent: "asia" },
  WS: { code: "SG", label: "Singapur", continent: "asia" },
  ZK: { code: "KP", label: "Corea del Norte", continent: "asia" },
  ZM: { code: "MN", label: "Mongolia", continent: "asia" },
  // África
  FA: { code: "ZA", label: "Sudáfrica", continent: "africa" },
  FM: { code: "MG", label: "Madagascar", continent: "africa" },
  FV: { code: "ZW", label: "Zimbabue", continent: "africa" },
  FL: { code: "ZM", label: "Zambia", continent: "africa" },
  FN: { code: "AO", label: "Angola", continent: "africa" },
  FQ: { code: "MZ", label: "Mozambique", continent: "africa" },
  HE: { code: "EG", label: "Egipto", continent: "africa" },
  HK: { code: "KE", label: "Kenia", continent: "africa" },
  HT: { code: "TZ", label: "Tanzania", continent: "africa" },
  HU: { code: "UG", label: "Uganda", continent: "africa" },
  HH: { code: "ER", label: "Eritrea", continent: "africa" },
  HA: { code: "ET", label: "Etiopía", continent: "africa" },
  HL: { code: "LY", label: "Libia", continent: "africa" },
  HS: { code: "SD", label: "Sudán", continent: "africa" },
  GM: { code: "MA", label: "Marruecos", continent: "africa" },
  DT: { code: "TN", label: "Túnez", continent: "africa" },
  DA: { code: "DZ", label: "Argelia", continent: "africa" },
  DN: { code: "NG", label: "Nigeria", continent: "africa" },
  DG: { code: "GH", label: "Ghana", continent: "africa" },
  DI: { code: "CI", label: "Costa de Marfil", continent: "africa" },
  GO: { code: "SN", label: "Senegal", continent: "africa" },
  GV: { code: "CV", label: "Cabo Verde", continent: "africa" },
  // Oceanía
  YM: { code: "AU", label: "Australia", continent: "oceania" },
  YS: { code: "AU", label: "Australia", continent: "oceania" },
  YB: { code: "AU", label: "Australia", continent: "oceania" },
  YP: { code: "AU", label: "Australia", continent: "oceania" },
  YW: { code: "AU", label: "Australia", continent: "oceania" },
  YL: { code: "AU", label: "Australia", continent: "oceania" },
  YC: { code: "AU", label: "Australia", continent: "oceania" },
  NZ: { code: "NZ", label: "Nueva Zelanda", continent: "oceania" },
  NF: { code: "FJ", label: "Fiyi", continent: "oceania" },
  NT: { code: "PF", label: "Polinesia Francesa", continent: "oceania" },
  NW: { code: "NC", label: "Nueva Caledonia", continent: "oceania" },
  AY: { code: "PG", label: "Papúa N. Guinea", continent: "oceania" },
};

// Fallback por inicial OACI cuando el país de 2 letras no está en la
// tabla. Incluye un `code` corto y legible (p. ej. K* y P* → "USA").
const REGION_1: Record<string, { code: string; label: string; continent: Continent }> = {
  K: { code: "USA", label: "EE.UU.", continent: "north_america" },
  C: { code: "CAN", label: "Canadá", continent: "north_america" },
  P: { code: "USA", label: "EE.UU. (Pacífico)", continent: "north_america" },
  M: { code: "C.AM", label: "México y Centroamérica", continent: "central_america" },
  T: { code: "CARIB", label: "Caribe", continent: "caribbean" },
  S: { code: "S.AM", label: "Sudamérica", continent: "south_america" },
  E: { code: "EUR", label: "Europa (Norte)", continent: "europe" },
  L: { code: "EUR", label: "Europa (Sur)", continent: "europe" },
  B: { code: "EUR", label: "Europa (Norte)", continent: "europe" },
  U: { code: "RU", label: "Rusia y CIS", continent: "europe" },
  R: { code: "ASIA", label: "Asia oriental", continent: "asia" },
  Z: { code: "CN", label: "China", continent: "asia" },
  V: { code: "ASIA", label: "Sur de Asia", continent: "asia" },
  W: { code: "ASIA", label: "Sudeste asiático", continent: "asia" },
  O: { code: "M.E", label: "Oriente Medio", continent: "middle_east" },
  F: { code: "AFR", label: "África (Sur/Centro)", continent: "africa" },
  G: { code: "AFR", label: "África (Oeste)", continent: "africa" },
  H: { code: "AFR", label: "África (Este)", continent: "africa" },
  D: { code: "AFR", label: "África (Oeste)", continent: "africa" },
  A: { code: "OC", label: "Pacífico Occidental", continent: "oceania" },
  N: { code: "OC", label: "Pacífico Sur", continent: "oceania" },
  Y: { code: "AUS", label: "Australia", continent: "oceania" },
};

const CONTINENT_LABEL: Record<Continent, string> = {
  north_america: "Norteamérica",
  central_america: "Centroamérica",
  caribbean: "Caribe",
  south_america: "Sudamérica",
  europe: "Europa",
  africa: "África",
  middle_east: "Oriente Medio",
  asia: "Asia",
  oceania: "Oceanía",
  other: "Otros",
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
    return { code: "?", label: "Otros", continent: "other" };
  }
  const two = c.slice(0, 2);
  const hit2 = COUNTRY_2[two];
  if (hit2) {
    return { code: hit2.code, label: hit2.label, continent: hit2.continent };
  }
  const hit1 = REGION_1[c[0]];
  if (hit1) {
    return { code: hit1.code, label: hit1.label, continent: hit1.continent };
  }
  return { code: c[0], label: "Otros", continent: "other" };
}

/** Etiqueta legible de la macro-zona (continente) — para agrupar. */
export function continentLabel(cont: Continent): string {
  return CONTINENT_LABEL[cont];
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
