import { create } from "zustand";
import { api, isTauri, type MaintAlert } from "../lib/tauri";

/**
 * (v6.2.19) Alertas de mantenimiento para la campanita: aviones con algún
 * componente >=75% de desgaste. Se refrescan al arrancar y cada vez que el
 * flight log cambia (aterrizaste → el desgaste subió).
 *
 * Descartes: por matrícula+componente en localStorage, guardando el desgaste
 * al descartar — la alerta REAPARECE si el desgaste sube >=5 puntos más (o si
 * el usuario hace el servicio y el componente vuelve a acumular).
 */

const DISMISS_KEY = "simfleet.maint.dismissed.v1";

type DismissMap = Record<string, number>; // "REG|component" -> wearPct al descartar

function loadDismissed(): DismissMap {
  try {
    const raw = localStorage.getItem(DISMISS_KEY);
    return raw ? (JSON.parse(raw) as DismissMap) : {};
  } catch {
    return {};
  }
}

function saveDismissed(map: DismissMap) {
  try {
    localStorage.setItem(DISMISS_KEY, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}

function alertKey(a: MaintAlert): string {
  return `${a.registration.toUpperCase()}|${a.component}`;
}

interface MaintAlertsState {
  alerts: MaintAlert[];
  dismissed: DismissMap;
  /** Alertas visibles (no descartadas, o cuyo desgaste creció tras descartar). */
  visible: () => MaintAlert[];
  refresh: () => Promise<void>;
  dismiss: (a: MaintAlert) => void;
  /** Arranca el ciclo: fetch inicial + re-fetch en cada cambio del flight log. */
  start: () => void;
}

let started = false;

export const useMaintAlertsStore = create<MaintAlertsState>((set, get) => ({
  alerts: [],
  dismissed: loadDismissed(),

  visible: () => {
    const { alerts, dismissed } = get();
    return alerts.filter((a) => {
      const at = dismissed[alertKey(a)];
      return at == null || a.wearPct >= at + 5;
    });
  },

  refresh: async () => {
    try {
      const alerts = await api.maintenanceAlerts();
      set({ alerts });
    } catch (e) {
      console.warn("maintenanceAlerts falló:", e);
    }
  },

  dismiss: (a) => {
    const next = { ...get().dismissed, [alertKey(a)]: a.wearPct };
    set({ dismissed: next });
    saveDismissed(next);
  },

  start: () => {
    if (started) return;
    started = true;
    void get().refresh();
    if (!isTauri) return;
    void import("@tauri-apps/api/event").then(({ listen }) => {
      void listen("flightlog://changed", () => void get().refresh());
    });
  },
}));
