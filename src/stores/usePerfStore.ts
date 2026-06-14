import { create } from "zustand";
import { api } from "../lib/tauri";

/**
 * (v5.0.0) Store del estado "optimizable" de los escenarios: qué
 * aeropuertos tienen objetos opcionales desactivables (autos, pasajeros,
 * GSE, clutter…) y por tanto pueden mostrar el badge de la tuerca + abrir
 * el modal de rendimiento.
 *
 * Se llena bajo demanda con `ensure(items)` desde las vistas que listan
 * aeropuertos (Map, Addons/Link Map). El escaneo se hace en el backend
 * (`perf_list_optimizable`, que mira el `installPath`) — así cubre
 * también los escenarios LOCALES que el usuario nunca volverá a
 * descargar. Cachea por `folderName` para no re-escanear lo ya visto.
 */
interface PerfState {
  /** folderName → true si tiene objetos opcionales. */
  optimizable: Set<string>;
  /** folderName ya escaneados (tengan o no opciones) — evita re-escaneo. */
  scanned: Set<string>;
  scanning: boolean;
  /** Escanea los que falten de la lista dada y actualiza el set. */
  ensure: (items: { folderName: string; installPath: string }[]) => Promise<void>;
  /** (v5.0.0) Escáner masivo: baja la nota "Optional Configuration" de
   *  SceneryAddons para todos los aeropuertos y marca los que la tengan.
   *  Devuelve cuántos quedaron optimizables y cuántos se intentaron. */
  scanAllFromSource: (
    items: {
      folderName: string;
      installPath: string;
      icao: string | null;
      creator: string | null;
    }[],
  ) => Promise<{ found: number; total: number }>;
  /** Marca uno como optimizable (p. ej. tras abrir el modal y hallar
   *  opciones vía SceneryAddons). */
  markOptimizable: (folderName: string) => void;
}

export const usePerfStore = create<PerfState>((set, get) => ({
  optimizable: new Set<string>(),
  scanned: new Set<string>(),
  scanning: false,
  async ensure(items) {
    const { scanned } = get();
    const pending = items.filter(
      (it) => it.folderName && it.installPath && !scanned.has(it.folderName),
    );
    if (pending.length === 0) return;
    set({ scanning: true });
    try {
      const hits = await api.perfListOptimizable(
        pending.map((it) => ({
          folderName: it.folderName,
          installPath: it.installPath,
        })),
      );
      set((s) => {
        const optimizable = new Set(s.optimizable);
        for (const f of hits) optimizable.add(f);
        const nextScanned = new Set(s.scanned);
        for (const it of pending) nextScanned.add(it.folderName);
        return { optimizable, scanned: nextScanned, scanning: false };
      });
    } catch (e) {
      console.warn("perfListOptimizable falló:", e);
      set({ scanning: false });
    }
  },
  async scanAllFromSource(items) {
    const list = items.filter((it) => it.folderName && it.installPath);
    if (list.length === 0) return { found: 0, total: 0 };
    set({ scanning: true });
    try {
      const hits = await api.perfScanAllFromSource(list);
      set((s) => {
        const optimizable = new Set(s.optimizable);
        for (const f of hits) optimizable.add(f);
        const scanned = new Set(s.scanned);
        for (const it of list) scanned.add(it.folderName);
        return { optimizable, scanned, scanning: false };
      });
      return { found: hits.length, total: list.length };
    } catch (e) {
      console.warn("perfScanAllFromSource falló:", e);
      set({ scanning: false });
      return { found: 0, total: list.length };
    }
  },
  markOptimizable(folderName) {
    set((s) => {
      if (s.optimizable.has(folderName)) return s;
      const optimizable = new Set(s.optimizable);
      optimizable.add(folderName);
      const scanned = new Set(s.scanned);
      scanned.add(folderName);
      return { optimizable, scanned };
    });
  },
}));
