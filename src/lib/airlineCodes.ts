// (v6.2.3) Base AUTO-GENERADA desde OpenFlights (~1180 aerolíneas) — cobertura
// amplia para que salga el logo de "todas" las aerolíneas, no sólo las del mapa
// curado. Los mapas curados de este archivo SIEMPRE tienen prioridad.
import {
  OF_ICAO_TO_IATA,
  OF_CALLSIGN_TO_IATA,
  OF_NAME_TO_IATA,
  OF_ICAO_TO_NAME,
} from "./airlineCodesData";

/**
 * (v4.9.0) Mapa ICAO (3 letras) → IATA (2 letras) de aerolíneas.
 *
 * Guardamos el código ICAO por vuelo (callsign/OFP), pero los CDN de
 * logos gratuitos (avs.io, daisycon) indexan por IATA. Este mapa CURADO
 * cubre las aerolíneas grandes + carriers modernos que faltan en
 * OpenFlights (p.ej. Arajet). Si un ICAO no está aquí, caemos a la base
 * de OpenFlights (`OF_ICAO_TO_IATA`); si tampoco está, AirlineLogo cae a
 * un chip con el código.
 *
 * Mantener en MAYÚSCULAS. Añadir entradas según haga falta.
 */
export const ICAO_TO_IATA: Record<string, string> = {
  // ─ Norteamérica ─
  AAL: "AA", // American Airlines
  ACA: "AC", // Air Canada
  ASA: "AS", // Alaska Airlines
  DAL: "DL", // Delta
  FFT: "F9", // Frontier
  HAL: "HA", // Hawaiian
  JBU: "B6", // JetBlue
  NKS: "NK", // Spirit
  SWA: "WN", // Southwest
  SCX: "SY", // Sun Country
  UAL: "UA", // United
  WJA: "WS", // WestJet
  AMX: "AM", // Aeroméxico
  VOI: "Y4", // Volaris
  VIV: "VB", // Viva Aerobus
  DWI: "DM", // Arajet (Rep. Dominicana) — no está en OpenFlights
  // ─ Latinoamérica ─
  LAN: "LA", // LATAM
  LPE: "LA", // LATAM Perú
  TAM: "LA", // LATAM Brasil
  ARG: "AR", // Aerolíneas Argentinas
  AVA: "AV", // Avianca
  AZU: "AD", // Azul
  GLO: "G3", // GOL
  CMP: "CM", // Copa
  SKU: "H2", // Sky Airline
  // ─ Europa (red) ─
  BAW: "BA", // British Airways
  SHT: "BA", // BA Shuttle
  AFR: "AF", // Air France
  HOP: "A5", // Air France Hop
  CCM: "XK", // Air Corsica
  CTV: "QG", // Citilink (Indonesia)
  MAU: "MK", // Air Mauritius
  DLH: "LH", // Lufthansa
  CLH: "LH", // Lufthansa CityLine
  KLM: "KL", // KLM
  IBE: "IB", // Iberia
  IBS: "I2", // Iberia Express
  ANE: "YW", // Air Nostrum
  AEA: "UX", // Air Europa
  SWR: "LX", // SWISS
  AUA: "OS", // Austrian
  BEL: "SN", // Brussels Airlines
  TAP: "TP", // TAP Portugal
  SAS: "SK", // Scandinavian (SAS)
  FIN: "AY", // Finnair
  ICE: "FI", // Icelandair
  AFL: "SU", // Aeroflot
  SBI: "S7", // S7 Airlines
  AUI: "PS", // Ukraine International
  LOT: "LO", // LOT Polish
  CSA: "OK", // Czech Airlines
  ROT: "RO", // TAROM
  AEE: "A3", // Aegean
  THY: "TK", // Turkish Airlines
  PGT: "PC", // Pegasus
  BTI: "BT", // airBaltic
  LGL: "LG", // Luxair
  VIR: "VS", // Virgin Atlantic
  // ─ Europa (low-cost) ─
  RYR: "FR", // Ryanair
  EZY: "U2", // easyJet
  EZS: "DS", // easyJet Switzerland
  EJU: "EC", // easyJet Europe
  WZZ: "W6", // Wizz Air
  VLG: "VY", // Vueling
  EWG: "EW", // Eurowings
  NAX: "DY", // Norwegian
  NSZ: "DY", // Norwegian (long-haul)
  TRA: "HV", // Transavia
  TVF: "TO", // Transavia France
  TVS: "QS", // Smartwings
  CFG: "DE", // Condor
  TOM: "BY", // TUI Airways
  JAF: "TB", // TUI fly Belgium
  TFL: "OR", // TUI fly Netherlands
  VOE: "V7", // Volotea
  // ─ Medio Oriente ─
  UAE: "EK", // Emirates
  ETD: "EY", // Etihad
  QTR: "QR", // Qatar Airways
  SVA: "SV", // Saudia
  GFA: "GF", // Gulf Air
  KAC: "KU", // Kuwait Airways
  OMA: "WY", // Oman Air
  RJA: "RJ", // Royal Jordanian
  MEA: "ME", // Middle East Airlines
  ELY: "LY", // El Al
  FDB: "FZ", // flydubai
  ABY: "G9", // Air Arabia
  JZR: "J9", // Jazeera Airways
  // ─ África ─
  MSR: "MS", // EgyptAir
  ETH: "ET", // Ethiopian
  KQA: "KQ", // Kenya Airways
  SAA: "SA", // South African
  RAM: "AT", // Royal Air Maroc
  DAH: "AH", // Air Algérie
  TAR: "TU", // Tunisair
  // ─ Asia (este) ─
  CCA: "CA", // Air China
  CES: "MU", // China Eastern
  CSN: "CZ", // China Southern
  CKK: "CK", // China Cargo Airlines
  TTW: "IT", // Tigerair Taiwan
  CHH: "HU", // Hainan Airlines
  CXA: "MF", // Xiamen Air
  CSC: "3U", // Sichuan Airlines
  CSZ: "ZH", // Shenzhen Airlines
  CDG: "SC", // Shandong Airlines
  CQH: "9C", // Spring Airlines
  CPA: "CX", // Cathay Pacific
  HDA: "UO", // HK Express
  CAL: "CI", // China Airlines
  EVA: "BR", // EVA Air
  SJX: "JX", // STARLUX
  JAL: "JL", // Japan Airlines
  ANA: "NH", // All Nippon Airways
  ADO: "HD", // AirDo
  SKY: "BC", // Skymark
  JJP: "GK", // Jetstar Japan
  KAL: "KE", // Korean Air
  AAR: "OZ", // Asiana
  ABL: "BX", // Air Busan
  JNA: "LJ", // Jin Air
  // ─ Asia (sur / sureste) ─
  AIC: "AI", // Air India
  AXB: "IX", // Air India Express
  IGO: "6E", // IndiGo
  VTI: "UK", // Vistara
  SEJ: "SG", // SpiceJet
  PIA: "PK", // Pakistan International
  BBC: "BG", // Biman Bangladesh
  ULL: "UL", // SriLankan
  THA: "TG", // Thai Airways
  BKP: "PG", // Bangkok Airways
  NOK: "DD", // Nok Air
  TGW: "TR", // Scoot
  SIA: "SQ", // Singapore Airlines
  SLK: "MI", // SilkAir
  MAS: "MH", // Malaysia Airlines
  AXM: "AK", // AirAsia
  GIA: "GA", // Garuda Indonesia
  LNI: "JT", // Lion Air
  BTK: "ID", // Batik Air
  CEB: "5J", // Cebu Pacific
  PAL: "PR", // Philippine Airlines
  HVN: "VN", // Vietnam Airlines
  VJC: "VJ", // VietJet Air
  UZB: "HY", // Uzbekistan Airways
  // ─ Oceanía ─
  QFA: "QF", // Qantas
  JST: "JQ", // Jetstar
  VOZ: "VA", // Virgin Australia
  ANZ: "NZ", // Air New Zealand
  // ─ Carga ─
  FDX: "FX", // FedEx
  UPS: "5X", // UPS Airlines
  GTI: "5Y", // Atlas Air
  CLX: "CV", // Cargolux
  NCA: "KZ", // Nippon Cargo
  CKS: "K4", // Kalitta Air
  DHL: "D0", // DHL (callsign WORLD EXPRESS; avs.io no tiene cargo → daisycon sí)
  BOX: "3S", // AeroLogic (DHL/Lufthansa)
  GEC: "LH", // Lufthansa Cargo
  ABW: "RU", // AirBridgeCargo
};

/** Devuelve el IATA para un ICAO conocido, o `null`. */
export function icaoToIata(icao?: string | null): string | null {
  if (!icao) return null;
  const k = icao.toUpperCase().trim();
  // Curado primero (mejor calidad / modernos), luego la base OpenFlights.
  return ICAO_TO_IATA[k] ?? OF_ICAO_TO_IATA[k] ?? null;
}

/**
 * (v4.9.2) Resolución por NOMBRE de aerolínea → IATA.
 *
 * Muchos vuelos importados (VAS-ACARS) no traen callsign ni código, así
 * que `airline_icao` queda NULL — pero sí tenemos el nombre/título del
 * avión (p.ej. "PMDG 737-800 Kenya Airways (5Y-CYC | 2015)" o
 * "iFly 737-MAX8 WestJet C-FEWJ (178Seat)"). Buscamos palabras clave
 * conocidas DENTRO de ese texto para resolver el logo.
 *
 * Lista ordenada: las claves más específicas (multi-palabra) van primero
 * para evitar falsos positivos (p.ej. "air india express" antes que
 * "air india"; "latam" antes que cualquier coincidencia parcial).
 */
const NAME_KEYWORD_TO_IATA: ReadonlyArray<readonly [string, string]> = [
  // multi-palabra / específicos primero
  ["air india express", "IX"],
  ["air india", "AI"],
  ["air france", "AF"],
  ["air corsica", "XK"],
  ["air canada", "AC"],
  ["air china", "CA"],
  ["air new zealand", "NZ"],
  ["air mauritius", "MK"],
  ["air europa", "UX"],
  ["air baltic", "BT"],
  ["air arabia", "G9"],
  ["air algerie", "AH"],
  ["air algérie", "AH"],
  ["air astana", "KC"],
  ["air serbia", "JU"],
  ["air transat", "TS"],
  ["air do", "HD"],
  ["airdo", "HD"],
  ["all nippon", "NH"],
  ["ana ", "NH"],
  ["american eagle", "AA"],
  ["american airlines", "AA"],
  ["british airways", "BA"],
  ["china eastern", "MU"],
  ["china southern", "CZ"],
  ["china airlines", "CI"],
  ["china united", "KN"],
  ["hainan", "HU"],
  ["xiamen", "MF"],
  ["sichuan", "3U"],
  ["shenzhen", "ZH"],
  ["shandong", "SC"],
  ["spring airlines", "9C"],
  ["cathay pacific", "CX"],
  ["cathay", "CX"],
  ["eva air", "BR"],
  ["starlux", "JX"],
  ["japan airlines", "JL"],
  ["skymark", "BC"],
  ["jetstar japan", "GK"],
  ["jetstar", "JQ"],
  ["korean air", "KE"],
  ["asiana", "OZ"],
  ["air busan", "BX"],
  ["jin air", "LJ"],
  ["kenya airways", "KQ"],
  ["ethiopian", "ET"],
  ["egyptair", "MS"],
  ["egypt air", "MS"],
  ["south african", "SA"],
  ["royal air maroc", "AT"],
  ["tunisair", "TU"],
  ["singapore airlines", "SQ"],
  ["singapore", "SQ"],
  ["silkair", "MI"],
  ["malaysia", "MH"],
  ["airasia", "AK"],
  ["garuda", "GA"],
  ["lion air", "JT"],
  ["batik", "ID"],
  ["cebu pacific", "5J"],
  ["cebu", "5J"],
  ["philippine", "PR"],
  ["vietnam airlines", "VN"],
  ["vietjet", "VJ"],
  ["uzbekistan", "HY"],
  ["thai airways", "TG"],
  ["thai ", "TG"],
  ["bangkok airways", "PG"],
  ["nok air", "DD"],
  ["scoot", "TR"],
  ["sri lankan", "UL"],
  ["srilankan", "UL"],
  ["pakistan", "PK"],
  ["biman", "BG"],
  ["indigo", "6E"],
  ["vistara", "UK"],
  ["spicejet", "SG"],
  ["tigerair", "TR"],
  ["citilink", "QG"],
  ["super air jet", "IU"],
  ["juneyao", "HO"],
  ["loong air", "GJ"],
  ["hong kong airlines", "HX"],
  ["greater bay", "HB"],
  ["bamboo", "QH"],
  ["pacific airlines", "BL"],
  ["myanmar", "8M"],
  ["royal brunei", "BI"],
  ["fiji airways", "FJ"],
  ["t'way", "TW"],
  ["tway air", "TW"],
  ["air premia", "YP"],
  ["jeju air", "7C"],
  ["jeju", "7C"],
  ["eastar", "ZE"],
  ["salam air", "OV"],
  ["flynas", "XY"],
  ["flyadeal", "F3"],
  ["nepal airlines", "RA"],
  // Norteamérica
  ["westjet", "WS"],
  ["delta", "DL"],
  ["united", "UA"],
  ["southwest", "WN"],
  ["jetblue", "B6"],
  ["spirit", "NK"],
  ["frontier", "F9"],
  ["alaska airlines", "AS"],
  ["hawaiian", "HA"],
  ["sun country", "SY"],
  ["aeromexico", "AM"],
  ["aeroméxico", "AM"],
  ["volaris", "Y4"],
  ["viva aerobus", "VB"],
  // Latinoamérica
  ["latam", "LA"],
  ["aerolineas argentinas", "AR"],
  ["aerolíneas argentinas", "AR"],
  ["avianca", "AV"],
  ["azul", "AD"],
  ["gol ", "G3"],
  ["copa", "CM"],
  ["sky airline", "H2"],
  // Europa
  ["lufthansa", "LH"],
  ["klm", "KL"],
  ["iberia express", "I2"],
  ["iberia", "IB"],
  ["air nostrum", "YW"],
  ["swiss", "LX"],
  ["austrian", "OS"],
  ["brussels", "SN"],
  ["tap air portugal", "TP"],
  ["tap portugal", "TP"],
  ["scandinavian", "SK"],
  ["sas ", "SK"],
  ["finnair", "AY"],
  ["icelandair", "FI"],
  ["aeroflot", "SU"],
  ["s7 airlines", "S7"],
  ["ukraine international", "PS"],
  ["lot polish", "LO"],
  ["lot ", "LO"],
  ["czech airlines", "OK"],
  ["tarom", "RO"],
  ["aegean", "A3"],
  ["turkish", "TK"],
  ["pegasus", "PC"],
  ["luxair", "LG"],
  ["virgin atlantic", "VS"],
  ["virgin australia", "VA"],
  ["ryanair", "FR"],
  ["easyjet", "U2"],
  ["wizz", "W6"],
  ["vueling", "VY"],
  ["eurowings", "EW"],
  ["norwegian", "DY"],
  ["transavia", "HV"],
  ["smartwings", "QS"],
  ["condor", "DE"],
  ["tui ", "BY"],
  ["volotea", "V7"],
  // Medio Oriente
  ["emirates", "EK"],
  ["etihad", "EY"],
  ["qatar", "QR"],
  ["saudia", "SV"],
  ["saudi arabian", "SV"],
  ["gulf air", "GF"],
  ["kuwait airways", "KU"],
  ["oman air", "WY"],
  ["royal jordanian", "RJ"],
  ["middle east airlines", "ME"],
  ["el al", "LY"],
  ["flydubai", "FZ"],
  ["jazeera", "J9"],
  // Oceanía / otros
  ["qantas", "QF"],
  ["aer lingus", "EI"],
  ["widerøe", "WF"],
  ["wideroe", "WF"],
  ["croatia airlines", "OU"],
  ["sunexpress", "XQ"],
  ["rwandair", "WB"],
  ["airlink", "4Z"],
  ["allegiant", "G4"],
  ["porter", "PD"],
  ["breeze", "MX"],
  ["play ", "OG"],
  // Carga
  ["fedex", "FX"],
  ["ups ", "5X"],
  ["atlas air", "5Y"],
  ["cargolux", "CV"],
];

/**
 * Intenta resolver el IATA desde un nombre/título libre de aerolínea.
 *
 * Ejemplos de título (MSFS): "PMDG 737-900ER DAL (N827DN - C.E.Woolman)",
 * "Fenix A320 - Tigerair 'Standard' 9V-TAR (2016)".
 *
 * Pasos:
 *  1. LIMPIEZA: quitamos paréntesis (regs/variantes/años) y descartamos
 *     tokens que parezcan **matrícula** o variante de avión — cualquiera
 *     con dígito o guion (A320, 737-900ER, 9V-TAR, PK-GQN, N827DN). Esto
 *     evita resolver por la matrícula (p.ej. "9V-TAR" contiene "TAR" =
 *     Tunisair → falso positivo).
 *  2. Palabra clave de nombre dentro del texto limpio.
 *  3. Último recurso: un token ICAO de 3 letras conocido embebido en el
 *     título (estilo PMDG, p.ej. "DAL" → DL, "UAE" → EK, "BAW" → BA).
 */
export function nameToIata(name?: string | null): string | null {
  if (!name) return null;
  const raw = name.toUpperCase().trim();
  const tokens = name
    .replace(/\([^)]*\)/g, " ") // (N827DN - C.E.Woolman), (2016), (178Seat)
    .split(/\s+/)
    .filter((t) => t && !/[0-9-]/.test(t)); // fuera matrículas/variantes
  const cleaned = tokens.join(" ");
  const hay = " " + cleaned.toLowerCase() + " ";

  // 1) Palabras clave curadas (resuelven el texto de TÍTULO de livery, p.ej.
  //    "iFly 737-MAX8 WestJet C-FEWJ" → WS).
  for (const [kw, iata] of NAME_KEYWORD_TO_IATA) {
    if (hay.includes(kw)) return iata;
  }
  // 2) (v6.2.3) Callsign EXACTO de OpenFlights — el `ATC AIRLINE` del sim suele
  //    SER el callsign ("COPA"→CM, "SOUTHWEST"→WN, "DOMINICANA"→DO).
  if (OF_CALLSIGN_TO_IATA[raw]) return OF_CALLSIGN_TO_IATA[raw];
  // 3) (v6.2.3) Nombre completo normalizado de OpenFlights ("Aerolane"→XL,
  //    "Vietnam Airlines"→VN) — para valores que son el nombre, no el callsign.
  const nm = raw.replace(/[^A-Z0-9]/g, "");
  if (nm.length >= 4 && OF_NAME_TO_IATA[nm]) return OF_NAME_TO_IATA[nm];
  // 4) Token ICAO de 3 letras embebido en el texto (curado o base OpenFlights).
  for (const tok of tokens) {
    const t = tok.toUpperCase().replace(/[^A-Z]/g, "");
    if (t.length === 3) {
      const iata = ICAO_TO_IATA[t] ?? OF_ICAO_TO_IATA[t];
      if (iata) return iata;
    }
  }
  return null;
}

/**
 * Resolución combinada: ICAO primero (callsign autoritativo), si falla
 * cae al nombre. Devuelve el IATA listo para el CDN de logos, o `null`.
 */
export function resolveIata(
  icao?: string | null,
  name?: string | null,
): string | null {
  return icaoToIata(icao) ?? nameToIata(name);
}

/**
 * (v5.3.4) Nombre de aerolínea a partir del ICAO. La fila "Airline" del
 * FlightBook quedaba vacía cuando el sim no reportaba `ATC AIRLINE` pero sí
 * teníamos el ICAO (del OFP/callsign — el que pinta el logo). Mapeamos los
 * carriers más comunes; si no está, el caller cae al propio código ICAO
 * (mejor que "—"). No pretende ser exhaustivo — cubre lo que la gente vuela.
 */
const ICAO_TO_NAME: Record<string, string> = {
  AAL: "American Airlines",
  DAL: "Delta Air Lines",
  UAL: "United Airlines",
  SWA: "Southwest Airlines",
  JBU: "JetBlue",
  ASA: "Alaska Airlines",
  NKS: "Spirit Airlines",
  FFT: "Frontier Airlines",
  HAL: "Hawaiian Airlines",
  RPA: "Republic Airways",
  SKW: "SkyWest",
  ACA: "Air Canada",
  WJA: "WestJet",
  BAW: "British Airways",
  VIR: "Virgin Atlantic",
  DLH: "Lufthansa",
  AFR: "Air France",
  KLM: "KLM",
  IBE: "Iberia",
  RYR: "Ryanair",
  EZY: "easyJet",
  VLG: "Vueling",
  EWG: "Eurowings",
  SWR: "SWISS",
  AUA: "Austrian Airlines",
  TAP: "TAP Air Portugal",
  SAS: "SAS",
  FIN: "Finnair",
  THY: "Turkish Airlines",
  UAE: "Emirates",
  QTR: "Qatar Airways",
  ETD: "Etihad Airways",
  SVA: "Saudia",
  ELY: "El Al",
  QFA: "Qantas",
  ANZ: "Air New Zealand",
  SIA: "Singapore Airlines",
  CPA: "Cathay Pacific",
  ANA: "All Nippon Airways",
  JAL: "Japan Airlines",
  KAL: "Korean Air",
  AAR: "Asiana Airlines",
  CCA: "Air China",
  CES: "China Eastern",
  CSN: "China Southern",
  AIC: "Air India",
  AVA: "Avianca",
  LAN: "LATAM",
  LPE: "LATAM Perú",
  TAM: "LATAM Brasil",
  AMX: "Aeroméxico",
  CMP: "Copa Airlines",
  VOI: "Volaris",
  VIV: "Viva Aerobus",
  ARG: "Aerolíneas Argentinas",
  GLO: "GOL",
  AZU: "Azul",
  AZA: "ITA Airways",
  DWI: "Arajet", // no está en OpenFlights
};

/** Nombre legible de la aerolínea por ICAO; `null` si no está mapeada. */
export function icaoToName(icao?: string | null): string | null {
  if (!icao) return null;
  const k = icao.toUpperCase().trim();
  // Curado primero; luego la base de OpenFlights (~1180 nombres reales).
  return ICAO_TO_NAME[k] ?? OF_ICAO_TO_NAME[k] ?? null;
}

/**
 * (v6.2.36) Clave CANÓNICA de aerolínea para deduplicar los chips del
 * FlightBook y hacer que el filtro cuadre todas las variantes. Prioriza
 * el `airline_icao` (TRIM/UPPER); si falta, extrae el primer token del
 * `aircraft_airline` limpiando un sufijo "(MATRÍCULA)". Así "DAL",
 * "DAL " y `aircraft_airline="DAL (N374DA)"` colapsan a la misma clave
 * "DAL" y sus vuelos se suman en un único logo.
 */
export function canonicalAirlineKey(
  icao?: string | null,
  name?: string | null,
): string {
  // (v6.2.37 B2) Resolvemos a IATA cuando se puede, para que fusionen
  // TODAS las variantes de una aerolínea: código ICAO, nombre completo
  // ("Delta Air Lines") y `aircraft_airline` sucio ("DAL (N374DA)").
  const i = (icao ?? "").trim().toUpperCase();
  if (i) {
    const iata = icaoToIata(i);
    return iata ? `I:${iata}` : `C:${i}`;
  }
  const cleaned = (name ?? "").replace(/\s*\([^)]*\)\s*$/, "").trim();
  if (!cleaned) return "";
  const tok = (cleaned.split(/[\s\-/]+/)[0] ?? "").toUpperCase();
  // Un token de 3 letras suele ser un código ICAO ("DAL") colado en el
  // nombre — resuélvelo como tal para que case con el icao real.
  if (/^[A-Z]{3}$/.test(tok)) {
    const iata = icaoToIata(tok);
    return iata ? `I:${iata}` : `C:${tok}`;
  }
  // Nombre completo → intenta IATA; si no, cae al primer token.
  const iata = nameToIata(cleaned);
  return iata ? `I:${iata}` : `N:${tok}`;
}
