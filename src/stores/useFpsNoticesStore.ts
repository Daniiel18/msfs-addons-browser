import { create } from "zustand";
import { api } from "../lib/tauri";
import { useCommunityStore } from "./useCommunityStore";
import { useSettingsStore } from "./useSettingsStore";

/**
 * (v6.2.20 · v6.2.23) Avisos de "FPS Optimization" para la campanita:
 *  · escenario NUEVO instalado con objetos opcionales → aviso;
 *  · escenario ACTUALIZADO (el update lo reinstala y BORRA las
 *    optimizaciones aplicadas) → aviso de re-optimizar.
 * El clic abre el modal de rendimiento (igual que la tuerca del Link Map).
 *
 * Mecánica: mapa carpeta→versión persistido. Carpeta desconocida = instalado
 * nuevo; versión distinta = update (las .bgl volvieron a default). En ambos
 * casos se escanea con `perf_list_optimizable` y si trae opciones se avisa.
 * La primera vez se SIEMBRA sin avisar (lo ya instalado no es "nuevo").
 */

const KNOWN_V1_KEY = "simfleet.fps.known.v1"; // legado: array de folders
const KNOWN_V2_KEY = "simfleet.fps.known.v2"; // legado: folder → version (sin sim)
// (v7.2.4) mapa "sim::folder" → packageVersion. La clave lleva el SIM porque el
// MISMO aeropuerto tiene versiones distintas en 2020 vs 2024; sin separarlo,
// cambiar de simulador se leía como "update" y disparaba avisos de re-optimizar
// en CADA cambio de sim / arranque (bug reportado).
const KNOWN_KEY = "simfleet.fps.known.v3";
const NOTICES_KEY = "simfleet.fps.notices.v1";

export interface FpsNotice {
  folderName: string;
  title: string;
  at: number;
  /** "installed" = escenario nuevo · "updated" = update lo reinstaló. */
  reason: "installed" | "updated";
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
  // Avisos legado (pre-v6.2.23) sin `reason` → se tratan como "installed".
  notices: loadJson<FpsNotice[]>(NOTICES_KEY, []).map((n) => ({
    ...n,
    reason: n.reason ?? ("installed" as const),
  })),

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
      // (v7.2.4) Baseline POR SIM: el mismo aeropuerto tiene distinta
      // packageVersion en 2020 vs 2024, así que la clave lleva el sim activo.
      const sim = useSettingsStore.getState().settings.simVersion || "unknown";
      checking = true;
      try {
        const verOf = (v: string | null | undefined) => v ?? "";
        const keyFor = (folder: string) => `${sim}::${folder}`;
        const known = loadJson<Record<string, string>>(KNOWN_KEY, {});
        const simPrefix = `${sim}::`;
        const simSeen = Object.keys(known).some((k) => k.startsWith(simPrefix));

        // Primera vez que vemos ESTE sim (arranque inicial o el PRIMER cambio a
        // él) → sembramos su baseline SIN avisar. Lo ya instalado no es "nuevo",
        // y las versiones distintas entre sims NO son un update real.
        if (!simSeen) {
          const seeded = { ...known };
          for (const p of pkgs) {
            seeded[keyFor(p.folderName)] = verOf(p.packageVersion);
          }
          saveJson(KNOWN_KEY, seeded);
          try {
            localStorage.removeItem(KNOWN_V1_KEY);
            localStorage.removeItem(KNOWN_V2_KEY);
          } catch {
            /* ignore */
          }
          return;
        }

        // Diff dentro de ESTE sim: carpetas nuevas (instalado) o con versión
        // distinta (update — la reinstalación borró las optimizaciones).
        const fresh: { pkg: (typeof pkgs)[number]; reason: FpsNotice["reason"] }[] =
          [];
        for (const p of pkgs) {
          if (!p.folderName || !p.installPath) continue;
          const prev = known[keyFor(p.folderName)];
          if (prev === undefined) {
            fresh.push({ pkg: p, reason: "installed" });
          } else if (prev !== verOf(p.packageVersion)) {
            fresh.push({ pkg: p, reason: "updated" });
          }
        }
        if (fresh.length > 0) {
          // Registra el estado nuevo ANTES de escanear (aunque no sean
          // optimizables, no queremos re-procesarlos).
          const nextKnown = { ...known };
          for (const f of fresh) {
            nextKnown[keyFor(f.pkg.folderName)] = verOf(f.pkg.packageVersion);
          }
          saveJson(KNOWN_KEY, nextKnown);

          const hits = await api.perfListOptimizable(
            fresh.map((f) => ({
              folderName: f.pkg.folderName,
              installPath: f.pkg.installPath,
            })),
          );
          if (hits.length > 0) {
            const hitSet = new Set(hits);
            const current = get().notices;
            const added: FpsNotice[] = [];
            for (const f of fresh) {
              if (!hitSet.has(f.pkg.folderName)) continue;
              added.push({
                folderName: f.pkg.folderName,
                title: f.pkg.title || f.pkg.folderName,
                at: Date.now(),
                reason: f.reason,
              });
            }
            if (added.length > 0) {
              // Un update reemplaza el aviso previo de esa carpeta (si lo había).
              const addedSet = new Set(added.map((n) => n.folderName));
              const next = [
                ...added,
                ...current.filter((n) => !addedSet.has(n.folderName)),
              ].slice(0, 20);
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
