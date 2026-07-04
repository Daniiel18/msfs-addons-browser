import { create } from "zustand";

/**
 * (v6.2.25) Recuerda qué optimizaciones FPS (ids de opción) tenías APLICADAS
 * por escenario (folderName). Cuando un update reinstala el escenario y borra
 * los renombrados de .bgl, esta memoria permite ofrecer "restaurar como estaba"
 * en un clic — el .bgl vuelve a default en el reinstall pero SimFleet sabe qué
 * tenías puesto.
 *
 * Fuente de verdad: se sincroniza en cada lectura de config que muestre
 * opciones aplicadas, y en cada toggle del usuario. NO se sobrescribe con "0
 * aplicadas" (que es justo el estado post-update), para no perder la memoria.
 */

const KEY = "simfleet.fps.applied.v1";

type AppliedMap = Record<string, string[]>; // folderName → applied option ids

function load(): AppliedMap {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as AppliedMap) : {};
  } catch {
    return {};
  }
}

function persist(map: AppliedMap) {
  try {
    localStorage.setItem(KEY, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}

interface FpsAppliedState {
  applied: AppliedMap;
  get: (folder: string) => string[];
  /** Sincroniza el set aplicado desde una config recién leída/togglada. Solo
   *  sobrescribe si hay ≥1 aplicada (evita borrar la memoria tras un update). */
  syncFromConfig: (folder: string, appliedIds: string[]) => void;
}

export const useFpsAppliedStore = create<FpsAppliedState>((set, get) => ({
  applied: load(),
  get: (folder) => get().applied[folder] ?? [],
  syncFromConfig: (folder, appliedIds) => {
    set((s) => {
      // Si el escenario acaba de reinstalarse (0 aplicadas) conservamos la
      // memoria anterior para poder ofrecer restaurar.
      if (appliedIds.length === 0 && (s.applied[folder]?.length ?? 0) > 0) {
        return s;
      }
      const next = { ...s.applied, [folder]: appliedIds };
      persist(next);
      return { applied: next };
    });
  },
}));
