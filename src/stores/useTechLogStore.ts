import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, isTauri } from "../lib/tauri";
import { t } from "../lib/i18n";
import type { PreflightMaintenance } from "../lib/preflight";
import type { TechLogPendingResult } from "../lib/types";
import { useToastStore } from "./useToastStore";

/**
 * (19/10) Etapa 2 — sincronía con Fenix/PMDG TechLog.
 *
 * `managed` = matrículas (normalizadas) que tiene algún TechLog: para ellas
 * TechLog es la fuente de verdad del mantenimiento (Hangar, FlightBook) y
 * SimFleet no aplica su propio desgaste. `epoch` sube cada vez que TechLog
 * avisa de un cambio (`techlog://changed`) para que los paneles se recarguen.
 */
interface TechLogState {
  managed: Set<string>;
  epoch: number;
  loaded: boolean;
  load: () => Promise<void>;
  isManaged: (registration: string | null | undefined) => boolean;
  /** (19/10) Despacho por matrícula normalizada (para el preflight). */
  dispatch: Record<string, PreflightMaintenance>;
  loadDispatch: (registration: string) => Promise<void>;
}

/** Igual que `techlog::normalize_reg` en Rust. */
export function normalizeTechLogReg(reg: string): string {
  const parts = reg.trim().split("-");
  const base =
    parts.length >= 3 && /^(CFM|IAE)$/i.test(parts[parts.length - 1])
      ? parts.slice(0, -2).join("-")
      : reg;
  return base.replace(/[^A-Za-z0-9]/g, "").toUpperCase();
}

const inflight = new Set<string>();

export const useTechLogStore = create<TechLogState>((set, get) => ({
  managed: new Set(),
  epoch: 0,
  loaded: false,
  async load() {
    try {
      const regs = await api.techlogManagedRegistrations();
      set({ managed: new Set(regs), loaded: true });
    } catch {
      set({ loaded: true });
    }
  },
  isManaged(registration) {
    if (!registration) return false;
    return get().managed.has(normalizeTechLogReg(registration));
  },
  dispatch: {},
  async loadDispatch(registration) {
    const key = normalizeTechLogReg(registration);
    const stamp = `${key}|${get().epoch}|${get().managed.size}`;
    if (inflight.has(stamp)) return;
    inflight.add(stamp);
    try {
      const a = get().managed.has(key)
        ? await api.techlogAircraft(registration)
        : await api.mxAircraft(registration);
      if (!a) return;
      set((s) => ({
        dispatch: {
          ...s.dispatch,
          [key]: {
            registration: a.registration,
            appName: a.appName,
            status: a.dispatch.status,
            reasons: a.dispatch.reasons,
          },
        },
      }));
    } catch {
      /* sin datos de mantenimiento: el preflight lo omite */
    }
  },
}));

let wired = false;
/** Carga inicial + escucha de cambios de TechLog (una sola vez). */
export function initTechLogSync() {
  if (wired) return;
  wired = true;
  void useTechLogStore.getState().load();
  if (!isTauri) return;
  void listen<string>("techlog://changed", () => {
    useTechLogStore.setState((s) => ({ epoch: s.epoch + 1 }));
    void useTechLogStore.getState().load();
  });
  // (19/10) Etapa 3: el Tech Log de SimFleet aplicó un vuelo.
  void listen("mx://changed", () => {
    useTechLogStore.setState((s) => ({ epoch: s.epoch + 1 }));
  });
  // (20/10) Etapa 4: TechLog aplicó órdenes que estaban en cola.
  void listen<TechLogPendingResult[]>("techlog://pending", (e) => {
    const done = e.payload ?? [];
    const ok = done.filter((r) => r.ok);
    const failed = done.filter((r) => !r.ok);
    const push = useToastStore.getState().push;
    if (ok.length > 0) {
      push({
        kind: "success",
        title: t("techlog.pending.applied_title"),
        message: ok.map((r) => `${r.registration}: ${r.label}`).join(" · "),
      });
    }
    for (const r of failed) {
      push({
        kind: "error",
        title: t("techlog.pending.failed_title"),
        message: `${r.registration}: ${r.label}${r.message ? ` — ${r.message}` : ""}`,
      });
    }
  });
}
