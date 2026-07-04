import { create } from "zustand";
import { api } from "../lib/tauri";
import type { Addon } from "../lib/types";

/**
 * (v6.2.26 / R3) Radar de escenarios NUEVOS del catálogo.
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
const DISMISS_KEY = "simfleet.newscenery.dismissed.v1";

function loadSeen(): number | null {
  const raw = localStorage.getItem(SEEN_KEY);
  if (!raw) return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
}
function saveSeen(ms: number) {
  try {
    localStorage.setItem(SEEN_KEY, String(ms));
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
  fresh: Addon[];
  loading: boolean;
  lastCheckedAt: number | null;
  /** Navega el catálogo y recalcula `fresh` respetando la versión. */
  check: (simVersion: string) => Promise<void>;
  /** Marca todo lo actual como visto → limpia `fresh`. */
  acknowledgeAll: () => void;
  /** Oculta un addon puntual del radar. */
  dismiss: (id: string) => void;
}

export const useNewSceneryStore = create<NewSceneryState>((set, get) => ({
  fresh: [],
  loading: false,
  lastCheckedAt: null,

  async check(simVersion) {
    if (get().loading) return;
    set({ loading: true });
    try {
      const page = await api.browseSource("sceneryaddons", 1);
      const dismissed = loadDismissed();
      const dated = page.addons
        .filter((a) => matchesSimVersion(a.simulator, simVersion))
        .map((a) => ({ a, ms: releasedMs(a) }))
        .filter((x): x is { a: Addon; ms: number } => x.ms !== null)
        .sort((x, y) => y.ms - x.ms);

      if (dated.length === 0) {
        set({ loading: false, lastCheckedAt: Date.now() });
        return;
      }

      const newestMs = dated[0].ms;
      const seen = loadSeen();
      if (seen === null) {
        // Primer arranque: sembramos el marcador, sin inundar.
        saveSeen(newestMs);
        set({ fresh: [], loading: false, lastCheckedAt: Date.now() });
        return;
      }

      const fresh = dated
        .filter((x) => x.ms > seen && !dismissed.has(x.a.id))
        .map((x) => x.a);
      set({ fresh, loading: false, lastCheckedAt: Date.now() });
    } catch (e) {
      console.warn("newScenery.check falló:", e);
      set({ loading: false });
    }
  },

  acknowledgeAll() {
    const { fresh } = get();
    if (fresh.length === 0) return;
    const newest = fresh
      .map((a) => Date.parse(a.releasedAt ?? ""))
      .filter((n) => Number.isFinite(n));
    if (newest.length) saveSeen(Math.max(...newest));
    set({ fresh: [] });
  },

  dismiss(id) {
    const d = loadDismissed();
    d.add(id);
    saveDismissed(d);
    set((s) => ({ fresh: s.fresh.filter((a) => a.id !== id) }));
  },
}));
