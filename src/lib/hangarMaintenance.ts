import type { HangarAircraft } from "./types";

/**
 * (v6.1 #31 #32) Mantenimiento SINTÉTICO del Hangar.
 *
 * No hay tracking real de mantenimiento; derivamos un historial y un estado de
 * componentes CREÍBLES a partir del uso real del avión (horas, ciclos = vuelos,
 * aterrizajes duros, salud del tren). Todo es DETERMINISTA por matrícula: el
 * mismo avión muestra siempre el mismo historial entre recargas.
 */

// PRNG determinista (mulberry32) sembrado por un hash de la matrícula/modelo.
function hashSeed(s: string): number {
  let h = 2166136261 >>> 0;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Talleres MRO ficticios (nombre + código + color de marca para el logo). */
const SHOPS = [
  { name: "AeroTech MRO", code: "ATM", color: "#0ea5e9" },
  { name: "SkyWrench Aviation", code: "SWA", color: "#f59e0b" },
  { name: "JetCare Services", code: "JCS", color: "#10b981" },
  { name: "Hangar 7 Maintenance", code: "H7M", color: "#a855f7" },
  { name: "Continental AeroCare", code: "CAC", color: "#ef4444" },
] as const;

const MECHANICS = [
  "M. Álvarez (AMT-2241)",
  "R. Okafor (AMT-1187)",
  "J. Tanaka (AMT-3390)",
  "L. Bianchi (AMT-0925)",
  "S. Novak (AMT-4412)",
];

export interface GearComponent {
  key: string;
  label: string;
  /** 0..100 desgaste (100 = nuevo invertido → mostramos salud = 100-wear). */
  healthPct: number;
  status: "good" | "watch" | "alert";
  /** Horas hasta el próximo servicio (puede ser negativo = vencido). */
  nextDueHours: number;
}

export interface ReportPart {
  name: string;
  partNumber: string;
  qty: number;
  reason: string;
  cost: number;
}

export interface MaintenanceReport {
  id: string;
  /** ISO date. */
  date: string;
  shop: { name: string; code: string; color: string };
  mechanic: string;
  hoursAtService: number;
  cyclesAtService: number;
  type: string; // "A-Check" | "Line" | "Unscheduled" | ...
  parts: ReportPart[];
  laborCost: number;
  partsCost: number;
  totalCost: number;
  summary: string;
  nextDueHours: number;
  nextDueDate: string;
}

export interface MaintenanceData {
  hours: number;
  cycles: number;
  components: GearComponent[];
  reports: MaintenanceReport[];
  /** Coste total acumulado de mantenimiento (suma de reportes). */
  lifetimeCost: number;
  nextServiceHours: number;
}

const statusOf = (health: number): GearComponent["status"] =>
  health >= 80 ? "good" : health >= 50 ? "watch" : "alert";

/** Catálogo de piezas por componente (para los reportes). */
const PARTS_BY_COMPONENT: Record<string, ReportPart[]> = {
  gear: [
    { name: "Juego de neumáticos (main)", partNumber: "TR-46x16", qty: 2, reason: "Desgaste por ciclos / aterrizajes duros", cost: 1850 },
    { name: "Pastillas de freno carbono", partNumber: "BRK-CC-9", qty: 4, reason: "Espesor bajo límite", cost: 2400 },
    { name: "Sello hidráulico tren", partNumber: "SEAL-LG-3", qty: 1, reason: "Fuga menor detectada", cost: 320 },
  ],
  engine: [
    { name: "Filtro de aceite", partNumber: "OIL-F-22", qty: 2, reason: "Servicio programado por horas", cost: 180 },
    { name: "Bujías / igniters", partNumber: "IGN-7B", qty: 2, reason: "Horas de operación", cost: 640 },
    { name: "Inspección boroscópica", partNumber: "SVC-BORO", qty: 1, reason: "A-Check de motor", cost: 1200 },
  ],
  hydraulics: [
    { name: "Fluido hidráulico Skydrol", partNumber: "HYD-5606", qty: 1, reason: "Cambio programado", cost: 260 },
    { name: "Bomba hidráulica (overhaul)", partNumber: "PMP-3000", qty: 1, reason: "Horas de servicio", cost: 3100 },
  ],
  avionics: [
    { name: "Batería de respaldo aviónica", partNumber: "BAT-AV-2", qty: 1, reason: "Fin de vida útil", cost: 540 },
    { name: "Actualización base de datos NAV", partNumber: "DB-NAV", qty: 1, reason: "Ciclo AIRAC", cost: 90 },
  ],
};

/**
 * Deriva el estado de componentes + un historial de mantenimiento sintético a
 * partir de las métricas reales del avión.
 */
export function deriveMaintenance(ac: HangarAircraft): MaintenanceData {
  const hours = Math.round((ac.totalTimeS ?? 0) / 3600);
  const cycles = ac.flights ?? 0;
  const rng = mulberry32(hashSeed(`${ac.registration ?? ac.model ?? ac.key}`));

  // ── Componentes ──────────────────────────────────────────────────────────
  const gearHealth = ac.healthPct; // ya derivado de aterrizajes duros
  const engineHealth = Math.max(20, 100 - Math.round((hours % 4000) / 40));
  const brakesHealth = Math.max(
    15,
    100 - cycles * 2 - (ac.hardLandings ?? 0) * 6,
  );
  const hydHealth = Math.max(30, 100 - Math.round((hours % 6000) / 80));
  const avHealth = Math.max(45, 100 - Math.round((hours % 8000) / 160));

  const components: GearComponent[] = [
    { key: "gear", label: "Tren de aterrizaje", healthPct: gearHealth, status: statusOf(gearHealth), nextDueHours: Math.round((gearHealth - 20) * 8) },
    { key: "engine", label: "Motores", healthPct: engineHealth, status: statusOf(engineHealth), nextDueHours: Math.round((engineHealth - 15) * 10) },
    { key: "brakes", label: "Frenos y neumáticos", healthPct: brakesHealth, status: statusOf(brakesHealth), nextDueHours: Math.round((brakesHealth - 10) * 4) },
    { key: "hydraulics", label: "Sistema hidráulico", healthPct: hydHealth, status: statusOf(hydHealth), nextDueHours: Math.round((hydHealth - 20) * 12) },
    { key: "avionics", label: "Aviónica", healthPct: avHealth, status: statusOf(avHealth), nextDueHours: Math.round((avHealth - 25) * 20) },
  ];

  // ── Historial de reportes ────────────────────────────────────────────────
  // Un evento cada ~120h de vuelo, hasta 6 eventos, repartidos hacia atrás en
  // el tiempo desde el último vuelo.
  const eventCount = Math.min(6, Math.max(1, Math.floor(hours / 120) + 1));
  const lastFlight = ac.lastFlightAt ? new Date(ac.lastFlightAt) : new Date();
  const reports: MaintenanceReport[] = [];
  const checkTypes = ["Line Check", "A-Check", "Unscheduled", "Service Bulletin"];

  for (let i = 0; i < eventCount; i++) {
    const shop = SHOPS[Math.floor(rng() * SHOPS.length)];
    const mechanic = MECHANICS[Math.floor(rng() * MECHANICS.length)];
    const hoursAtService = Math.max(0, hours - i * Math.round(80 + rng() * 80));
    const cyclesAtService = Math.max(0, cycles - i * Math.round(8 + rng() * 10));
    // Elegimos 1-3 componentes "intervenidos" para este evento.
    const compKeys = Object.keys(PARTS_BY_COMPONENT);
    const picked = compKeys
      .filter(() => rng() > 0.45)
      .slice(0, 3);
    const useKeys = picked.length ? picked : [compKeys[Math.floor(rng() * compKeys.length)]];
    const parts: ReportPart[] = [];
    for (const k of useKeys) {
      const pool = PARTS_BY_COMPONENT[k];
      const p = pool[Math.floor(rng() * pool.length)];
      // pequeña variación de coste determinista
      parts.push({ ...p, cost: Math.round(p.cost * (0.9 + rng() * 0.3)) });
    }
    const partsCost = parts.reduce((s, p) => s + p.cost * p.qty, 0);
    const laborCost = Math.round((300 + rng() * 1200) + parts.length * 240);
    const date = new Date(
      lastFlight.getTime() - i * (1000 * 60 * 60 * 24 * Math.round(18 + rng() * 30)),
    );
    const nextDueHours = Math.round(100 + rng() * 200);
    const nextDueDate = new Date(
      date.getTime() + 1000 * 60 * 60 * 24 * Math.round(60 + rng() * 120),
    );
    reports.push({
      id: `${ac.key}-mx-${i}`,
      date: date.toISOString(),
      shop,
      mechanic,
      hoursAtService,
      cyclesAtService,
      type: i === 0 ? checkTypes[0] : checkTypes[Math.floor(rng() * checkTypes.length)],
      parts,
      laborCost,
      partsCost,
      totalCost: laborCost + partsCost,
      summary:
        useKeys
          .map((k) => components.find((c) => c.key === k)?.label ?? k)
          .join(", ") + " — inspección y reemplazo de componentes según límites.",
      nextDueHours,
      nextDueDate: nextDueDate.toISOString(),
    });
  }

  const lifetimeCost = reports.reduce((s, r) => s + r.totalCost, 0);
  const nextServiceHours = Math.min(...components.map((c) => c.nextDueHours));

  return { hours, cycles, components, reports, lifetimeCost, nextServiceHours };
}
