/**
 * (v4.9.0) Mapa ICAO (3 letras) → IATA (2 letras) de aerolíneas.
 *
 * Guardamos el código ICAO por vuelo (callsign/OFP), pero los CDN de
 * logos gratuitos (avs.io, daisycon) indexan por IATA. Este mapa cubre
 * las aerolíneas grandes del mundo; si un ICAO no está aquí, el
 * componente AirlineLogo cae a un chip con el código (no pide red, así
 * evitamos el placeholder genérico que devuelven los CDN para códigos
 * desconocidos).
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
};

/** Devuelve el IATA para un ICAO conocido, o `null`. */
export function icaoToIata(icao?: string | null): string | null {
  if (!icao) return null;
  return ICAO_TO_IATA[icao.toUpperCase().trim()] ?? null;
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
  // Carga
  ["fedex", "FX"],
  ["ups ", "5X"],
  ["atlas air", "5Y"],
  ["cargolux", "CV"],
];

/**
 * Intenta resolver el IATA buscando palabras clave de aerolínea dentro
 * de un nombre/título libre. Devuelve el IATA o `null`.
 */
export function nameToIata(name?: string | null): string | null {
  if (!name) return null;
  const hay = name.toLowerCase();
  for (const [kw, iata] of NAME_KEYWORD_TO_IATA) {
    if (hay.includes(kw)) return iata;
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
