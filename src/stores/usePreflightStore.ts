import { create } from "zustand";

/**
 * (v6.2.34) Estado del pre-vuelo "¿Listo para volar?".
 *
 * · `open` — el modal se muestra a nivel App (para poder auto-abrirlo al
 *   detectar el OFP del día desde cualquier pantalla).
 * · `bypass` — ICAOs marcados como "usaré el escenario por defecto";
 *   persistido, para que ese punto no vuelva a marcarse como pendiente.
 * · `lastAlertedOfp` — id del último OFP para el que sonó la alerta +
 *   foco, persistido para no repetir el aviso del mismo plan.
 */

const BYPASS_KEY = "simfleet.preflight.bypass.v1";
const ALERTED_KEY = "simfleet.preflight.alerted_ofp.v1";

function loadBypass(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(BYPASS_KEY);
    return raw ? (JSON.parse(raw) as Record<string, boolean>) : {};
  } catch {
    return {};
  }
}
function persistBypass(map: Record<string, boolean>) {
  try {
    localStorage.setItem(BYPASS_KEY, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}

interface PreflightState {
  open: boolean;
  bypass: Record<string, boolean>;
  lastAlertedOfp: string | null;
  setOpen: (v: boolean) => void;
  setBypass: (icao: string, val: boolean) => void;
  setLastAlertedOfp: (id: string) => void;
}

export const usePreflightStore = create<PreflightState>((set) => ({
  open: false,
  bypass: loadBypass(),
  lastAlertedOfp: (() => {
    try {
      return localStorage.getItem(ALERTED_KEY);
    } catch {
      return null;
    }
  })(),
  setOpen: (v) => set({ open: v }),
  setBypass: (icao, val) =>
    set((s) => {
      const next = { ...s.bypass, [icao.toUpperCase()]: val };
      if (!val) delete next[icao.toUpperCase()];
      persistBypass(next);
      return { bypass: next };
    }),
  setLastAlertedOfp: (id) => {
    try {
      localStorage.setItem(ALERTED_KEY, id);
    } catch {
      /* ignore */
    }
    set({ lastAlertedOfp: id });
  },
}));
