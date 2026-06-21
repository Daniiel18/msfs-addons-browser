import type { FlightLogEntry } from "./types";

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

const PHASES: Array<{ at: number; phase: string; n1: number; alt: number; egt: number }> = [
  { at: 0, phase: "Taxi", n1: 0.25, alt: 0.0, egt: 0.4 },
  { at: 8, phase: "Despegue", n1: 0.98, alt: 0.02, egt: 1.0 },
  { at: 22, phase: "Ascenso", n1: 0.9, alt: 0.6, egt: 0.92 },
  { at: 40, phase: "Crucero", n1: 0.82, alt: 1.0, egt: 0.8 },
  { at: 70, phase: "Crucero", n1: 0.82, alt: 1.0, egt: 0.8 },
  { at: 84, phase: "Descenso", n1: 0.42, alt: 0.55, egt: 0.55 },
  { at: 95, phase: "Aproximación", n1: 0.55, alt: 0.12, egt: 0.62 },
  { at: 100, phase: "Aterrizaje", n1: 0.3, alt: 0.0, egt: 0.5 },
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
  const engines = engineCount(f.aircraftModel ?? f.aircraftAtcType ?? f.aircraftTitle);
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
      phase: p.phase,
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
