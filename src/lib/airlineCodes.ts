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
