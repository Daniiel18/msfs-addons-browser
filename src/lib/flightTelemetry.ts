import type { FlightLogEntry } from "./types";
import { t } from "./i18n";

/**
 * (v6.1) Telemetría SINTÉTICA por vuelo para el detalle desplegable del Hangar.
 *
 * El flight_log guarda agregados reales (altitud máx, velocidad máx, distancia,
 * combustible, FPM) pero NO series de tiempo de motores. Aquí derivamos una
 * curva de EGT/N1 CREÍBLE y zonas de calor a partir de esos agregados, de forma
 * DETERMINISTA por vuelo (mismo vuelo → misma curva). Entrelaza con la salud
 * del motor del mantenimiento: peor salud = más calor.
 */

function seed(s: string): number {
  let h = 2166136261 >>> 0;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}
function rng(s: number): () => number {
  let a = s >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Nº de motores por modelo (4 en quad clásicos, si no 2). */
export function engineCount(model: string | null): number {
  const m = (model ?? "").toUpperCase();
  if (/747|A340|A380|MD11|DC10|DC-10|IL96|B52|C5|C-5|A124/.test(m)) return 4;
  if (/C150|C152|C172|C182|PA28|PA-28|DA40|SR2|TBM|M20|DV20/.test(m)) return 1;
  return 2;
}

/** Motor real por tipo (nombre comercial + factor de tamaño del fan). */
export function engineModel(model: string | null): { name: string; fan: number; turboprop: boolean } {
  const m = (model ?? "").toUpperCase();
  const mk = (name: string, fan: number, turboprop = false) => ({ name, fan, turboprop });
  if (/7M8|7M9|7M7|B38M|B39M|MAX|737-?8|737-?9/.test(m)) return mk("CFM LEAP-1B", 1.0);
  if (/737|73[0-9]|B73|717|BBJ/.test(m)) return mk("CFM56-7B", 0.82);
  if (/A20N|A21N|A19N|NEO/.test(m)) return mk("LEAP-1A / PW1100G", 0.95);
  if (/A318|A319|A320|A321|A32/.test(m)) return mk("CFM56-5B / V2500", 0.8);
  if (/777|B777|A35K|A350-?1000/.test(m)) return mk("GE90 / GE9X", 1.3);
  if (/787|B787/.test(m)) return mk("GEnx-1B / Trent 1000", 1.15);
  if (/A350|A359/.test(m)) return mk("Trent XWB", 1.15);
  if (/A330|A33/.test(m)) return mk("Trent 700 / CF6", 1.05);
  if (/A340|A34/.test(m)) return mk("CFM56-5C", 0.85);
  if (/747|B747/.test(m)) return mk("GEnx-2B / CF6", 1.0);
  if (/A380|A388/.test(m)) return mk("Trent 900 / GP7200", 1.1);
  if (/757|B757/.test(m)) return mk("RB211 / PW2000", 0.92);
  if (/767|B767|MD11|DC10/.test(m)) return mk("CF6 / PW4000", 1.0);
  if (/CRJ|ERJ|E17|E19|E13|E14|RJ/.test(m)) return mk("CF34", 0.6);
  if (/DH8|DHC|Q400|ATR|AT4|AT7|SF34|D328/.test(m)) return mk("PW100 / PW150", 0.5, true);
  if (/TBM|C208|PC12|KODIAK/.test(m)) return mk("PT6", 0.42, true);
  if (/C150|C152|C172|C182|PA28|PA-28|DA40|SR2|M20|DV20/.test(m)) return mk(t("flightTelemetry.enginePiston"), 0.38, true);
  return mk("CFM56", 0.85);
}

export interface TelemetryPoint {
  /** % del vuelo (0..100). */
  t: number;
  phase: string;
  altFt: number;
  speedKt: number;
  n1: number; // %
  egt1: number; // °C motor 1
  egt2: number; // °C motor 2
}

export interface FlightTelemetry {
  engines: number;
  engineName: string;
  /** Factor de tamaño del fan (perspectiva del dibujo). */
  fan: number;
  turboprop: boolean;
  cruiseAltFt: number;
  cruiseSpeedKt: number;
  maxSpeedKt: number;
  avgN1: number;
  maxEgtC: number;
  fuelKg: number | null;
  /** Intensidad de la zona caliente 0..100 (más alto = más caliente). */
  heat: number;
  series: TelemetryPoint[];
}

// `phaseKey` como clave i18n — se traduce al construir la serie (render-time),
// no aquí a nivel de módulo (que corre antes de fijar el locale).
const PHASES: Array<{ at: number; phaseKey: string; n1: number; alt: number; egt: number }> = [
  { at: 0, phaseKey: "flightTelemetry.phaseTaxi", n1: 0.25, alt: 0.0, egt: 0.4 },
  { at: 8, phaseKey: "flightTelemetry.phaseTakeoff", n1: 0.98, alt: 0.02, egt: 1.0 },
  { at: 22, phaseKey: "flightTelemetry.phaseClimb", n1: 0.9, alt: 0.6, egt: 0.92 },
  { at: 40, phaseKey: "flightTelemetry.phaseCruise", n1: 0.82, alt: 1.0, egt: 0.8 },
  { at: 70, phaseKey: "flightTelemetry.phaseCruise", n1: 0.82, alt: 1.0, egt: 0.8 },
  { at: 84, phaseKey: "flightTelemetry.phaseDescent", n1: 0.42, alt: 0.55, egt: 0.55 },
  { at: 95, phaseKey: "flightTelemetry.phaseApproach", n1: 0.55, alt: 0.12, egt: 0.62 },
  { at: 100, phaseKey: "flightTelemetry.phaseLanding", n1: 0.3, alt: 0.0, egt: 0.5 },
];

/**
 * @param engineHealth salud del motor (0..100) del mantenimiento — modula el
 * calor (peor salud = EGT y zona caliente más altos).
 */
export function deriveTelemetry(
  f: FlightLogEntry,
  engineHealth = 100,
): FlightTelemetry {
  const r = rng(seed(`${f.id}-${f.aircraftRegistration ?? ""}`));
  const mdl = f.aircraftModel ?? f.aircraftAtcType ?? f.aircraftTitle;
  const engines = engineCount(mdl);
  const em = engineModel(mdl);
  const cruiseAltFt = Math.round((f.maxAltitudeFt ?? 34000 + r() * 6000) * 0.97);
  const maxSpeedKt = Math.round(f.maxTrueAirspeedKt ?? f.maxGroundSpeedKt ?? 440 + r() * 60);
  const cruiseSpeedKt = Math.round(maxSpeedKt * (0.92 + r() * 0.05));
  // Más calor si el motor está desgastado o si fue un vuelo exigente.
  const wear = Math.max(0, 100 - engineHealth);
  const heat = Math.min(100, Math.round(45 + wear * 0.5 + r() * 12));
  const maxEgtC = Math.round(620 + wear * 1.6 + r() * 40); // °C pico (despegue)
  const avgN1 = Math.round(80 + r() * 6);

  const series: TelemetryPoint[] = PHASES.map((p) => {
    const jitter = (amp: number) => (r() - 0.5) * amp;
    const egtBase = maxEgtC * p.egt;
    return {
      t: p.at,
      phase: t(p.phaseKey),
      altFt: Math.round(cruiseAltFt * p.alt),
      speedKt: Math.round(maxSpeedKt * Math.min(1, 0.15 + p.alt * 0.9)),
      n1: Math.round(p.n1 * 100 + jitter(3)),
      egt1: Math.round(egtBase + jitter(18)),
      egt2:
        engines > 1
          ? Math.round(egtBase + jitter(18) + (r() - 0.5) * 22)
          : 0,
    };
  });

  return {
    engines,
    engineName: em.name,
    fan: em.fan,
    turboprop: em.turboprop,
    cruiseAltFt,
    cruiseSpeedKt,
    maxSpeedKt,
    avgN1,
    maxEgtC,
    fuelKg: f.fuelUsedKg,
    heat,
    series,
  };
}
