import { create } from "zustand";
import { api } from "../lib/tauri";
import { useCommunityStore } from "./useCommunityStore";

/**
 * (v6.2.20) Avisos de "FPS Optimization disponible" para la campanita: cuando
 * se INSTALA un escenario nuevo (por descarga, drag&drop o manual) que tiene
 * objetos opcionales desactivables, se avisa y el clic abre el modal de
 * rendimiento — igual que la tuerca del Link Map.
 *
 * Mecánica: diff del listado de Community. La primera vez se SIEMBRA el set de
 * carpetas conocidas sin avisar (para no inundar con lo ya instalado); después,
 * cada carpeta NUEVA se escanea con `perf_list_optimizable` y si tiene
 * opciones genera un aviso persistente (localStorage) hasta descartarlo.
 */

const KNOWN_KEY = "simfleet.fps.known.v1";
const NOTICES_KEY = "simfleet.fps.notices.v1";

export interface FpsNotice {
  folderName: string;
  title: string;
  at: number;
}

function loadJson<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

function saveJson(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* ignore */
  }
}

interface FpsNoticesState {
  notices: FpsNotice[];
  dismiss: (folderName: string) => void;
  /** Suscribe el diff al listado de Community. Idempotente. */
  start: () => void;
}

let started = false;
let checking = false;

export const useFpsNoticesStore = create<FpsNoticesState>((set, get) => ({
  notices: loadJson<FpsNotice[]>(NOTICES_KEY, []),

  dismiss: (folderName) => {
    const next = get().notices.filter((n) => n.folderName !== folderName);
    set({ notices: next });
    saveJson(NOTICES_KEY, next);
  },

  start: () => {
    if (started) return;
    started = true;

    const check = async () => {
      if (checking) return;
      const pkgs = useCommunityStore.getState().packages;
      if (pkgs.length === 0) return;
      checking = true;
      try {
        const known = new Set(loadJson<string[]>(KNOWN_KEY, []));
        if (known.size === 0) {
          // Primera pasada: sembrar sin avisar (lo ya instalado no es "nuevo").
          saveJson(
            KNOWN_KEY,
            pkgs.map((p) => p.folderName),
          );
          return;
        }
        const fresh = pkgs.filter(
          (p) => p.folderName && p.installPath && !known.has(p.folderName),
        );
        // Registra TODAS las nuevas como conocidas (aunque no sean optimizables).
        if (fresh.length > 0) {
          saveJson(KNOWN_KEY, [
            ...known,
            ...fresh.map((p) => p.folderName),
          ]);
          const hits = await api.perfListOptimizable(
            fresh.map((p) => ({
              folderName: p.folderName,
              installPath: p.installPath,
            })),
          );
          if (hits.length > 0) {
            const hitSet = new Set(hits);
            const existing = new Set(get().notices.map((n) => n.folderName));
            const added: FpsNotice[] = fresh
              .filter(
                (p) => hitSet.has(p.folderName) && !existing.has(p.folderName),
              )
              .map((p) => ({
                folderName: p.folderName,
                title: p.title || p.folderName,
                at: Date.now(),
              }));
            if (added.length > 0) {
              const next = [...added, ...get().notices].slice(0, 20);
              set({ notices: next });
              saveJson(NOTICES_KEY, next);
            }
          }
        }
      } catch (e) {
        console.warn("fps notices check falló:", e);
      } finally {
        checking = false;
      }
    };

    // Chequeo inicial + en cada cambio del listado de paquetes.
    void check();
    let lastPkgs = useCommunityStore.getState().packages;
    useCommunityStore.subscribe((s) => {
      if (s.packages !== lastPkgs) {
        lastPkgs = s.packages;
        void check();
      }
    });
  },
}));
