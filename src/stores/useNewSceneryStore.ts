import { create } from "zustand";
import { api } from "../lib/tauri";
import type { Addon } from "../lib/types";

/**
 * (v6.2.26 / R3) Radar de escenarios NUEVOS del catálogo. (26/10) Y de lo
 * nuevo en AeroHub y Simplaza (aviones, escenarios, utilidades…), cada
 * catálogo con su propia marca de "visto".
 *
 * Al arrancar (y bajo demanda) navega la primera página de SceneryAddons
 * y compara las fechas de publicación con un marcador local: los addons
 * publicados DESPUÉS de la última vez que el usuario "vio las novedades"
 * son fresh. Respeta la versión de MSFS preferida (2020/2024) filtrando
 * por el campo `simulator`.
 *
 * Persistimos dos cosas en localStorage:
 *   · SEEN_KEY  — timestamp (ms) del addon más reciente ya reconocido.
 *   · DISMISS   — ids de addons ocultados manualmente.
 *
 * En el PRIMER arranque (sin marcador) NO inundamos: fijamos el marcador
 * al más reciente visto y `fresh` queda vacío. A partir de ahí, sólo lo
 * genuinamente nuevo aparece.
 */

const SEEN_KEY = "simfleet.newscenery.seen_ms.v1";

/** (26/10) Catálogos que mira el radar, con su marca de "visto"
 *  (SceneryAddons conserva la de siempre). */
const RADAR_SOURCES: { id: string; seenKey: string }[] = [
  { id: "sceneryaddons", seenKey: SEEN_KEY },
  { id: "aerohub", seenKey: "simfleet.newcatalog.aerohub.seen_ms.v1" },
  { id: "simplaza", seenKey: "simfleet.newcatalog.simplaza.seen_ms.v1" },
];
const DISMISS_KEY = "simfleet.newscenery.dismissed.v1";
/** (26/10) La primera vez, AeroHub y Simplaza cuentan la última semana. */
const FIRST_LOOK_MS = 7 * 24 * 3600 * 1000;

function loadSeen(key: string): number | null {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(key);
  } catch {
    return null;
  }
  if (!raw) return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
}
function saveSeen(key: string, ms: number) {
  try {
    localStorage.setItem(key, String(ms));
  } catch {
    /* ignore */
  }
}
function loadDismissed(): Set<string> {
  try {
    const raw = localStorage.getItem(DISMISS_KEY);
    return raw ? new Set(JSON.parse(raw) as string[]) : new Set();
  } catch {
    return new Set();
  }
}
function saveDismissed(s: Set<string>) {
  try {
    localStorage.setItem(DISMISS_KEY, JSON.stringify([...s]));
  } catch {
    /* ignore */
  }
}

/** ¿Coincide el addon con la versión de MSFS preferida? El campo viene
 *  como "MSFS 2020", "MSFS 2024" o "MSFS 2020/2024". Sin preferencia
 *  (setting vacío) → aceptamos todo. */
function matchesSimVersion(sim: string, pref: string): boolean {
  if (!pref) return true;
  const s = sim.toLowerCase();
  if (pref === "msfs2024") return s.includes("2024");
  // msfs2020: acepta 2020 explícito y el compartido 2020/2024.
  return s.includes("2020");
}

function releasedMs(a: Addon): number | null {
  if (!a.releasedAt) return null;
  const ms = Date.parse(a.releasedAt);
  return Number.isFinite(ms) ? ms : null;
}

interface NewSceneryState {
  /** Lo nuevo de todos los catálogos, lo más reciente primero. */
  fresh: Addon[];
  loading: boolean;
  lastCheckedAt: number | null;
  /** Navega los catálogos y recalcula `fresh` respetando la versión. */
  check: (simVersion: string) => Promise<void>;
  /** Marca todo lo actual como visto → limpia `fresh`. */
  acknowledgeAll: () => void;
  /** Oculta un addon puntual del radar. */
  dismiss: (id: string) => void;
}

/** Lo nuevo de UN catálogo desde su marca de "visto" (la siembra la primera
 *  vez, sin inundar). */
async function freshIn(source: { id: string; seenKey: string }, simVersion: string, dismissed: Set<string>) {
  const page = await api.browseSource(source.id, 1);
  // (26/10) AeroHub y Simplaza ya llegan filtrados por el MSFS elegido (un
  // addon de 2020 vale en 2024); SceneryAddons, como siempre, aquí.
  const dated = page.addons
    .filter((a) => source.id !== "sceneryaddons" || matchesSimVersion(a.simulator, simVersion))
    .map((a) => ({ a, ms: releasedMs(a) }))
    .filter((x): x is { a: Addon; ms: number } => x.ms !== null)
    .sort((x, y) => y.ms - x.ms);
  if (dated.length === 0) return [];
  let seen = loadSeen(source.seenKey);
  if (seen === null) {
    // Primer arranque: SceneryAddons siembra el marcador en lo más nuevo
    // (sin inundar); (26/10) AeroHub y Simplaza, en hace una semana, para que
    // la primera vez se vea lo último que entró.
    seen = source.id === "sceneryaddons" ? dated[0].ms : Date.now() - FIRST_LOOK_MS;
    saveSeen(source.seenKey, seen);
  }
  const mark = seen;
  return dated.filter((x) => x.ms > mark && !dismissed.has(x.a.id));
}

export const useNewSceneryStore = create<NewSceneryState>((set, get) => ({
  fresh: [],
  loading: false,
  lastCheckedAt: null,

  async check(simVersion) {
    if (get().loading) return;
    set({ loading: true });
    const dismissed = loadDismissed();
    // Cada catálogo por su lado: si uno falla, los demás siguen.
    const lists = await Promise.all(
      RADAR_SOURCES.map((src) =>
        freshIn(src, simVersion, dismissed).catch((e) => {
          console.warn(`newScenery.check(${src.id}) falló:`, e);
          return [];
        }),
      ),
    );
    const fresh = lists
      .flat()
      .sort((x, y) => y.ms - x.ms)
      .map((x) => x.a);
    set({ fresh, loading: false, lastCheckedAt: Date.now() });
  },

  acknowledgeAll() {
    const { fresh } = get();
    if (fresh.length === 0) return;
    // La marca de cada catálogo avanza hasta lo más nuevo que se vio de él.
    for (const src of RADAR_SOURCES) {
      const newest = fresh
        .filter((a) => a.source === src.id)
        .map((a) => releasedMs(a))
        .filter((n): n is number => n !== null);
      if (newest.length) saveSeen(src.seenKey, Math.max(...newest));
    }
    set({ fresh: [] });
  },

  dismiss(id) {
    const d = loadDismissed();
    d.add(id);
    saveDismissed(d);
    set((s) => ({ fresh: s.fresh.filter((a) => a.id !== id) }));
  },
}));
