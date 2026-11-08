import { derivedType, isAirport } from "./packageType";
import type {
  AddonLink,
  CommunityPackage,
  ContentSet,
  DispatchMode,
  DispatchTrip,
  LiveryEntry,
  SceneryChoice,
} from "./types";

/**
 * (03/11) Etapa 7 — qué necesita un vuelo y qué cambia cada modo. Usa la misma
 * clasificación que Instalados, para que «aviones» y «aeropuertos» signifiquen
 * lo mismo en toda la app.
 */

/** El vuelo que se va a lanzar. */
export interface DispatchFlight {
  /** La elección de avión vale para este vuelo, no para los siguientes. */
  key: string;
  origin: string | null;
  destination: string | null;
  aircraft: string | null;
  /** (04/11) Paradas intermedias de un viaje. */
  stops: string[];
  /** (03/11) Alternativos del OFP. */
  alternates: string[];
  /** (03/11) Matrícula del OFP. */
  registration: string | null;
  /** (05/11) Aerolínea del indicativo del OFP. */
  airline: string | null;
}

/** «WJA617» → «WJA». La misma regla que `derive_airline_icao`. */
function airlineOf(callsign: string | null | undefined): string | null {
  const text = callsign?.trim().toUpperCase() ?? "";
  return /^[A-Z]{3}[A-Z0-9]/.test(text) ? text.slice(0, 3) : null;
}

export function dispatchFlight(
  origin: string | null | undefined,
  destination: string | null | undefined,
  aircraft: string | null | undefined,
  /** El OFP, si el vuelo viene de él: de ahí salen alternativo y matrícula. */
  ofp?: { alternateIcaos?: string[]; aircraftReg?: string | null; callsign?: string | null } | null,
): DispatchFlight | null {
  const clean = (v: string | null | undefined) => v?.trim().toUpperCase() || null;
  const o = clean(origin);
  const d = clean(destination);
  const a = clean(aircraft);
  if (!o && !d) return null;
  return {
    key: `${o ?? ""}-${d ?? ""}-${a ?? ""}`,
    origin: o,
    destination: d,
    aircraft: a,
    stops: [],
    alternates: ofp?.alternateIcaos ?? [],
    registration: clean(ofp?.aircraftReg),
    airline: airlineOf(ofp?.callsign),
  };
}

/** (04/11) Códigos de aeropuerto de un texto: separados como se quiera. */
export const parseStops = (text: string): string[] =>
  text
    .toUpperCase()
    .split(/[^A-Z0-9]+/)
    .filter((code) => code.length >= 3 && code.length <= 5);

/** (04/11) Un viaje se prepara como un vuelo con paradas en medio. */
export function tripFlight(trip: DispatchTrip): DispatchFlight {
  const { stops, aircraft } = trip;
  return {
    key: `viaje-${stops.join("-")}-${aircraft ?? ""}`,
    origin: stops[0] ?? null,
    destination: stops[stops.length - 1] ?? null,
    aircraft,
    stops: stops.slice(1, -1),
    alternates: [],
    registration: null,
    airline: null,
  };
}

/** (05/11) Lo que mira la condición de un enlace. */
export interface FlightFacts {
  airports: string[];
  countries: string[];
  aircraft: string | null;
  airline: string | null;
}

/** Salida, paradas y llegada. El alternativo no cuenta: el plan no es volar a él. */
export const flightAirports = (flight: DispatchFlight): string[] =>
  [flight.origin, ...flight.stops, flight.destination].filter((code): code is string => !!code);

/** `countries`: ICAO → país. Un aeropuerto desconocido no aporta país. */
export function flightFacts(flight: DispatchFlight, countries: Record<string, string>): FlightFacts {
  const airports = flightAirports(flight);
  return {
    airports,
    countries: airports.map((code) => countries[code]).filter(Boolean),
    aircraft: flight.aircraft,
    airline: flight.airline,
  };
}

/** Dentro de un tipo basta con uno; entre tipos, hay que cumplirlos todos. */
export function linkApplies(link: AddonLink, facts: FlightFacts): boolean {
  const c = link.conditions;
  if (!c) return true;
  const anyOf = (wanted: string[], have: (string | null)[]) =>
    wanted.length === 0 || wanted.some((value) => have.includes(value));
  return (
    anyOf(c.country, facts.countries) &&
    anyOf(c.airport, facts.airports) &&
    anyOf(c.aircraft, [facts.aircraft]) &&
    anyOf(c.airline, [facts.airline])
  );
}

/** Elección «vuelo con un avión del simulador, sin addon». */
export const SIM_AIRCRAFT = "-";

export type FocusKind = "airport" | "aircraft" | "livery";

const key = (folder: string) => folder.toLowerCase();
const isOn = (p: CommunityPackage) => p.enabled !== false;
/** Sin manifest no hay `layout.json` que renombrar: no se puede apagar. */
const canToggle = (p: CommunityPackage) => p.hasManifest !== false;

function parseList(json: string | undefined): string[] {
  if (!json) return [];
  try {
    const v: unknown = JSON.parse(json);
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

/** Lo único que Enfocar puede apagar: aeropuertos, aviones y liveries. */
export function focusKind(p: CommunityPackage): FocusKind | null {
  if (!canToggle(p)) return null;
  if (isAirport(p)) return "airport";
  const d = derivedType(p);
  if (d === "AIRCRAFT") return "aircraft";
  if (d === "LIVERY") return "livery";
  return null;
}

/**
 * Escenarios de un aeropuerto. `icao` guarda un solo código por paquete: los
 * packs con varios aeropuertos (`…-airports-tncm-tffg-…`) sólo dicen el resto
 * en el nombre.
 */
export function sceneriesFor(code: string, packages: CommunityPackage[]): CommunityPackage[] {
  const icao = code.toUpperCase();
  const own = packages.filter((p) => p.icao?.toUpperCase() === icao && !p.isLibraryPack);
  if (own.length > 0 || icao.length !== 4) return own;
  const word = icao.toLowerCase();
  return packages.filter(
    (p) =>
      focusKind(p) === "airport" &&
      `${p.title} ${p.folderName}`.toLowerCase().split(/[^a-z0-9]+/).includes(word),
  );
}

/** Avión de verdad: los que se pueden elegir para volar. */
export function isPlane(p: CommunityPackage): boolean {
  return canToggle(p) && !!p.hasFlightModel && derivedType(p) === "AIRCRAFT";
}

// Tipo ICAO del OFP → cómo se nombra ese avión en título, carpeta o contenedor.
// Por variante donde cada una es un paquete (737, 777); por familia en el resto.
const TYPE_TEXT: [RegExp, RegExp][] = [
  [/^A318$/, /\ba318\b/],
  [/^A(319|19N)$/, /\ba319\b|\ba19n\b/],
  [/^A(320|20N)$/, /\ba320\b|\ba20n\b|\ba32nx\b/],
  [/^A(321|21N)$/, /\ba321\b|\ba21n\b/],
  [/^A33/, /\ba33[0-9n]\b/],
  [/^A34/, /\ba34[0-9]\b/],
  [/^A35/, /\ba35[0-9k]\b/],
  [/^A38/, /\ba38[0-9]x?\b/],
  [/^BCS/, /\ba220\b|\bcs[13]00\b|\bbcs[13]\b/],
  [/^B736$/, /\b737-?600\b|\bb?736\b/],
  [/^B737$/, /\b737-?700\b|\bb737\b|\baircraft-737\b/],
  [/^B738$/, /\b737-?800\b|\bb?738\b/],
  [/^B739$/, /\b737-?900\b|\bb?739\b/],
  [/^B37M$/, /\b737[ -]?max[ -]?7|\bb37m\b/],
  [/^B38M$/, /\b737[ -]?max[ -]?8|\b737[ -]?max\b|\bb38m\b/],
  [/^B39M$/, /\b737[ -]?max[ -]?9|\bb39m\b/],
  [/^B3XM$/, /\b737[ -]?max[ -]?10|\bb3xm\b/],
  [/^B74/, /\b747\b|\bb?74[478]\b/],
  [/^B75/, /\b757\b|\bb?75[23]\b/],
  [/^B76/, /\b767\b|\bb?76[234]\b/],
  [/^B772$/, /\b777-?200(er)?\b|\bb?772\b|\b77er\b/],
  [/^B77L$/, /\b777-?200lr\b|\bb?77l\b/],
  [/^B773$/, /\b777-?300\b|\bb?773\b/],
  [/^B77W$/, /\b777-?300er\b|\bb?77w\b/],
  [/^B77F$/, /\b777-?f\b|\bb?77f\b/],
  [/^B78/, /\b787\b|\bb?78[89x]\b/],
  [/^(E17|E75)/, /\be1?7[05x]\b|\be75[sl]\b/],
  [/^(E19|E29)/, /\be19[05x]\b|\be29[05]\b/],
  [/^CRJ/, /\bcrj\b|\bcrj[ -]?\d{3,4}\b/],
  [/^AT[47]/, /\batr\b|\bat[47][256]\b/],
  [/^DH8/, /\bdhc?[ -]?8|\bq400\b/],
  [/^MD1/, /\bmd-?11\b/],
  [/^MD[89]/, /\bmd-?8[0-9]\b|\bmd-?90\b/],
];

function typePattern(type: string): RegExp | null {
  const clean = type.replace(/[^A-Z0-9]/g, "");
  if (!clean) return null;
  return TYPE_TEXT.find(([t]) => t.test(clean))?.[1] ?? new RegExp(`\\b${clean.toLowerCase()}\\b`);
}

/** Aviones instalados que sirven para ese tipo del OFP. */
export function aircraftCandidates(type: string | null, packages: CommunityPackage[]): CommunityPackage[] {
  if (!type) return [];
  const pattern = typePattern(type);
  if (!pattern) return [];
  return packages
    .filter((p) => {
      if (!isPlane(p)) return false;
      const dirs = parseList(p.simobjectDirsJson);
      if (pattern.test(`${p.title} ${p.folderName} ${dirs.join(" ")}`.toLowerCase())) return true;
      // Sólo con modelo propio: los ayudantes también declaran un tipo.
      return !!p.hasOwnModel && parseList(p.icaoTypesJson).includes(type);
    })
    .sort((a, b) => Number(isOn(b)) - Number(isOn(a)) || a.title.localeCompare(b.title));
}

/**
 * Lo que necesita un avión para volar. No basta con el Link Map: los paquetes
 * comunes (Fenix) no tienen enlace, y sus enlaces automáticos por familia unen
 * aviones de marcas distintas.
 *
 *  1. Lo que comparte o hereda su contenedor de SimObjects: paquete común,
 *     pinturas y sonidos. Las utilidades (AICopilot) quedan como estén.
 *  2. Enlaces manuales, siempre: los puso el usuario.
 *  3. Enlaces automáticos, sólo hacia paquetes sin contenedor propio: lo que
 *     tiene contenedor y no coincide es de otro avión.
 *
 * (05/11) Una condición sin cumplir manda sobre el contenedor compartido: ese
 * addon no viene con el avión, y por ahí se corta la cadena.
 */
export function aircraftNeeds(
  aircraft: CommunityPackage,
  packages: CommunityPackage[],
  links: AddonLink[],
  facts: FlightFacts,
): CommunityPackage[] {
  const mine = new Set(parseList(aircraft.simobjectDirsJson).map(key));
  const containers = (p: CommunityPackage) =>
    [...parseList(p.simobjectDirsJson), ...parseList(p.baseContainersJson)].map(key);
  const byFolder = new Map(packages.map((p) => [key(p.folderName), p]));
  const out = new Map([[key(aircraft.folderName), aircraft]]);
  const held = new Set(links.filter((l) => !linkApplies(l, facts)).map((l) => key(l.targetFolder)));

  for (const p of packages) {
    if (derivedType(p) === "INSTRUMENT") continue;
    const directlyNeeded = containers(p).some((c) => mine.has(c));
    if (!directlyNeeded && held.has(key(p.folderName))) continue;
    if (directlyNeeded) out.set(key(p.folderName), p);
  }

  const queue = [...out.values()];
  while (queue.length > 0) {
    const current = queue.pop() as CommunityPackage;
    for (const l of links) {
      if (key(l.sourceFolder) !== key(current.folderName)) continue;
      const target = byFolder.get(key(l.targetFolder));
      if (!target || out.has(key(target.folderName)) || !linkApplies(l, facts)) continue;
      if (l.origin !== "manual" && containers(target).length > 0) continue;
      out.set(key(target.folderName), target);
      queue.push(target);
    }
  }
  // Lo que el avión declara necesitar, en sentido contrario.
  for (const l of links) {
    if (l.origin === "manual" || key(l.targetFolder) !== key(aircraft.folderName)) continue;
    if (!linkApplies(l, facts)) continue;
    const source = byFolder.get(key(l.sourceFolder));
    if (source && containers(source).length === 0) out.set(key(source.folderName), source);
  }
  return [...out.values()];
}

/**
 * (05/11) Enlaces con condición que salen de `sources`. El destino de uno que se
 * cumple trae también lo que el usuario le enlazó; lo demás que cuelga de un
 * addon encendido ya no es cosa del vuelo.
 */
function followConditions(
  sources: CommunityPackage[],
  packages: CommunityPackage[],
  links: AddonLink[],
  facts: FlightFacts,
): { met: CommunityPackage[]; unmet: CommunityPackage[] } {
  const met = new Map<string, CommunityPackage>();
  const unmet = new Map<string, CommunityPackage>();
  const byFolder = new Map(packages.map((p) => [key(p.folderName), p]));
  const seen = new Set(sources.map((p) => key(p.folderName)));
  const queue = [...sources];
  while (queue.length > 0) {
    const current = queue.pop() as CommunityPackage;
    const broughtIn = met.has(key(current.folderName));
    for (const l of links) {
      if (key(l.sourceFolder) !== key(current.folderName)) continue;
      const target = byFolder.get(key(l.targetFolder));
      if (!target || (!l.conditions && !(broughtIn && l.origin === "manual"))) continue;
      const k = key(target.folderName);
      if (!linkApplies(l, facts)) {
        unmet.set(k, target);
        continue;
      }
      met.set(k, target);
      if (!seen.has(k)) {
        seen.add(k);
        queue.push(target);
      }
    }
  }
  return { met: [...met.values()], unmet: [...unmet.values()].filter((p) => !met.has(key(p.folderName))) };
}

/** Sin signos ni espacios: «C-GORP» y «CGORP» son la misma matrícula. */
const normalizeReg = (reg: string) => reg.replace(/[^A-Za-z0-9]/g, "").toUpperCase();

/**
 * Hay pinturas que no declaran matrícula: queda el nombre. «B-304N» llega partido
 * en dos trozos y «CESB304N» pegado a la aerolínea.
 */
function nameHasRegistration(p: CommunityPackage, reg: string): boolean {
  const words = `${p.title} ${p.folderName}`.toUpperCase().split(/[^A-Z0-9]+/).filter(Boolean);
  return words.some(
    (w, i) => w === reg || `${w}${words[i + 1] ?? ""}` === reg || (reg.length >= 5 && w.endsWith(reg)),
  );
}

interface FoundLivery {
  pkg: CommunityPackage;
  title: string;
}

/** Dónde está la livery con esa matrícula exacta. */
function liveriesWith(registration: string, liveries: LiveryEntry[], packages: CommunityPackage[]): FoundLivery[] {
  const reg = normalizeReg(registration);
  if (!reg) return [];
  const byFolder = new Map(packages.map((p) => [key(p.folderName), p]));
  const found = new Map<string, FoundLivery>();
  for (const l of liveries) {
    const pkg = byFolder.get(key(l.folder));
    if (pkg && l.registration === reg) found.set(key(pkg.folderName), { pkg, title: l.title });
  }
  if (found.size === 0) {
    for (const p of packages) {
      if (focusKind(p) === "livery" && nameHasRegistration(p, reg)) found.set(key(p.folderName), { pkg: p, title: p.title });
    }
  }
  return [...found.values()];
}

/** El avión de esa livery: primero entre los que sirven para el tipo del OFP. */
function liveryOwner(
  found: FoundLivery[],
  candidates: CommunityPackage[],
  packages: CommunityPackage[],
  links: AddonLink[],
  facts: FlightFacts,
): CommunityPackage | undefined {
  if (found.length === 0) return undefined;
  const owns = (plane: CommunityPackage) => {
    const needs = aircraftNeeds(plane, packages, links, facts);
    return found.some((l) => needs.includes(l.pkg));
  };
  return candidates.find(owns) ?? packages.filter(isPlane).find(owns);
}

export interface NeedRow {
  role: "departure" | "stop" | "arrival" | "alternate" | "aircraft" | "livery";
  /** ICAO del aeropuerto, tipo del avión o matrícula. */
  code: string | null;
  pkg: CommunityPackage | null;
  /** Nombre de la livery, si no es el del paquete. */
  title?: string;
  /** (05/11) Escenarios encendidos entre los que hay que elegir. */
  clash?: CommunityPackage[];
  /**
   * `on` encendido · `turns_on` se enciende · `none` sin addon · `choose` falta
   * elegir · `missing` no tienes esa livery · `other` la livery es de otro avión.
   */
  status: "on" | "turns_on" | "none" | "choose" | "missing" | "other";
  candidates: CommunityPackage[];
}

export interface DispatchPlan {
  rows: NeedRow[];
  on: CommunityPackage[];
  off: CommunityPackage[];
  /** Se apagarían, pero están en «Nunca apagar». */
  kept: CommunityPackage[];
  /** (05/11) Fuera de este vuelo porque su enlace pide otra cosa. */
  skipped: CommunityPackage[];
  needsChoice: boolean;
  /** Addons del conjunto que ya no están instalados. */
  missing: number;
}

export const EMPTY_PLAN: DispatchPlan = {
  rows: [],
  on: [],
  off: [],
  kept: [],
  skipped: [],
  needsChoice: false,
  missing: 0,
};

interface FlightInput {
  mode: DispatchMode;
  flight: DispatchFlight;
  packages: CommunityPackage[];
  links: AddonLink[];
  keep: string[];
  liveries: LiveryEntry[];
  useAlternate: boolean;
  /** Avión elegido para este vuelo: carpeta o `SIM_AIRCRAFT`. */
  pick: string | undefined;
  /** (05/11) Escenario recordado por aeropuerto. */
  sceneryChoices?: SceneryChoice[];
  /** (05/11) País de los aeropuertos del vuelo, para las condiciones. */
  countries?: Record<string, string>;
}

export function planFlight({
  mode,
  flight,
  packages,
  links,
  keep,
  liveries,
  useAlternate,
  pick,
  sceneryChoices = [],
  countries = {},
}: FlightInput): DispatchPlan {
  const facts = flightFacts(flight, countries);
  const needed = new Map<string, CommunityPackage>();
  const need = (p: CommunityPackage) => needed.set(key(p.folderName), p);
  const rowStatus = (p: CommunityPackage | null): NeedRow["status"] => (!p ? "none" : isOn(p) ? "on" : "turns_on");

  // (05/11) Con varios encendidos decide el usuario, y Solo lanzar no toca nada.
  const prepares = mode === "add" || mode === "focus";
  const clashing: CommunityPackage[] = [];
  let sceneryUndecided = false;
  const airportRow = (role: "departure" | "stop" | "arrival" | "alternate", code: string | null): NeedRow => {
    const all = code ? sceneriesFor(code, packages) : [];
    const lit = all.filter(isOn);
    const choice = sceneryChoices.find((c) => c.icao === code?.toUpperCase());
    const clash = prepares && lit.length > 1 && !choice?.ignored;
    const chosen = clash ? lit.find((p) => key(p.folderName) === key(choice?.folder ?? "")) : undefined;
    const use = chosen ? [chosen] : lit.length > 0 ? lit : all.slice(0, 1);
    use.forEach(need);
    if (chosen) clashing.push(...lit.filter((p) => p !== chosen));
    if (clash && !chosen) sceneryUndecided = true;
    return {
      role,
      code,
      pkg: use[0] ?? null,
      status: clash && !chosen ? "choose" : rowStatus(use[0] ?? null),
      candidates: [],
      clash: clash ? lit : undefined,
    };
  };

  const candidates = aircraftCandidates(flight.aircraft, packages);
  const simAircraft = pick === SIM_AIRCRAFT;
  const found = flight.registration && !simAircraft ? liveriesWith(flight.registration, liveries, packages) : [];
  const picked = pick && !simAircraft ? packages.find((p) => p.folderName === pick) : undefined;
  // Una matrícula sólo está en un avión: si la tienes, ese es el avión.
  const owner = pick ? undefined : liveryOwner(found, candidates, packages, links, facts);
  const chosen = picked ?? owner ?? (!pick && candidates.length === 1 ? candidates[0] : undefined);
  const needs = chosen ? aircraftNeeds(chosen, packages, links, facts) : [];
  const livery = found.find((l) => needs.includes(l.pkg));
  // Con la livery del vuelo localizada, las demás de ese avión sobran.
  needs.filter((p) => !livery || p === livery.pkg || focusKind(p) !== "livery").forEach(need);

  // Una parada repetida o igual que la salida sólo necesita una fila.
  const stops = [...new Set(flight.stops)].filter((code) => code !== flight.origin && code !== flight.destination);
  const rows = [
    airportRow("departure", flight.origin),
    ...stops.map((code) => airportRow("stop", code)),
    airportRow("arrival", flight.destination),
  ];
  if (useAlternate) {
    flight.alternates
      .filter((code) => code !== flight.origin && code !== flight.destination)
      .forEach((code) => rows.push(airportRow("alternate", code)));
  }
  // Con «Añadir» sólo hay que preguntar si la respuesta cambia algo.
  const decides =
    mode === "focus" ||
    (mode === "add" && candidates.some((c) => aircraftNeeds(c, packages, links, facts).some((p) => !isOn(p))));
  const undecided = !pick && !chosen;
  const needsChoice = undecided && decides && (mode === "focus" || candidates.length > 1);
  rows.push({
    role: "aircraft",
    code: flight.aircraft,
    pkg: chosen ?? null,
    status: needsChoice ? "choose" : rowStatus(chosen ?? null),
    candidates,
  });
  if (flight.registration && !simAircraft) {
    rows.push({
      role: "livery",
      code: flight.registration,
      pkg: livery?.pkg ?? null,
      title: livery?.title,
      status: livery ? rowStatus(livery.pkg) : found.length > 0 ? "other" : "missing",
      candidates: [],
    });
  }

  if (mode !== "add" && mode !== "focus") return { ...EMPTY_PLAN, rows };

  const protect = new Set(keep.map(key));
  // (05/11) Un enlace con condición sale del avión o de lo que sigue encendido.
  const stays = (p: CommunityPackage) =>
    isOn(p) && (mode !== "focus" || focusKind(p) === null || protect.has(key(p.folderName)));
  const { met, unmet } = followConditions([...needed.values(), ...packages.filter(stays)], packages, links, facts);
  met.forEach(need);
  const skipped = unmet.filter((p) => !needed.has(key(p.folderName)));
  const on = [...needed.values()].filter((p) => !isOn(p) && canToggle(p));
  const rest =
    mode === "focus"
      ? [
          ...new Set([
            ...packages.filter((p) => isOn(p) && focusKind(p) !== null && !needed.has(key(p.folderName))),
            ...skipped.filter((p) => isOn(p) && canToggle(p)),
          ]),
        ]
      : clashing.filter((p, i) => !needed.has(key(p.folderName)) && canToggle(p) && clashing.indexOf(p) === i);
  return {
    rows,
    on,
    off: rest.filter((p) => !protect.has(key(p.folderName))),
    kept: rest.filter((p) => protect.has(key(p.folderName))),
    skipped,
    needsChoice: needsChoice || sceneryUndecided,
    missing: 0,
  };
}

/** (05/11) Aeropuertos que cubre un paquete: el suyo y los de un pack, que van en
 *  el nombre de la carpeta (`…-airports-tncm-tffg-…`). */
export function airportCodes(p: CommunityPackage): string[] {
  const codes = p.icao ? [p.icao.toUpperCase()] : [];
  const pack = p.folderName.toLowerCase().match(/-airports-((?:[a-z0-9]{4}-)+)/);
  for (const code of pack?.[1].split("-").filter(Boolean) ?? []) {
    if (!codes.includes(code.toUpperCase())) codes.push(code.toUpperCase());
  }
  return codes;
}

/** (05/11) Aeropuertos con más de un escenario encendido, salvo los que no hay
 *  que avisar. */
export function duplicateSceneries(
  packages: CommunityPackage[],
  choices: SceneryChoice[],
): Map<string, CommunityPackage[]> {
  const ignored = new Set(choices.filter((c) => c.ignored).map((c) => c.icao));
  const byCode = new Map<string, CommunityPackage[]>();
  for (const p of packages) {
    if (p.isLibraryPack || !isOn(p) || !isAirport(p)) continue;
    for (const code of airportCodes(p)) byCode.set(code, [...(byCode.get(code) ?? []), p]);
  }
  for (const [code, list] of byCode) {
    if (list.length < 2 || ignored.has(code)) byCode.delete(code);
  }
  return byCode;
}

/** Cambios para dejarlo todo como en el conjunto. Lo instalado después no se toca. */
export function planSet(set: ContentSet, packages: CommunityPackage[], keep: string[]): DispatchPlan {
  const enabled = new Set(set.enabled.map(key));
  const known = new Set(set.known.map(key));
  const protect = new Set(keep.map(key));
  const installed = new Set(packages.map((p) => key(p.folderName)));
  const plan: DispatchPlan = { ...EMPTY_PLAN, rows: [], on: [], off: [], kept: [] };
  for (const p of packages) {
    const k = key(p.folderName);
    if (!known.has(k) || !canToggle(p)) continue;
    if (enabled.has(k) && !isOn(p)) plan.on.push(p);
    else if (!enabled.has(k) && isOn(p)) (protect.has(k) ? plan.kept : plan.off).push(p);
  }
  plan.missing = set.known.filter((f) => !installed.has(key(f))).length;
  return plan;
}

export const folders = (list: CommunityPackage[]) => list.map((p) => p.folderName);
