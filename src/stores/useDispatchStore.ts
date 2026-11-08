import { useEffect, useMemo } from "react";
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, isTauri } from "../lib/tauri";
import { t } from "../lib/i18n";
import {
  EMPTY_PLAN,
  flightAirports,
  folders,
  planFlight,
  planSet,
  tripFlight,
  type DispatchFlight,
  type DispatchPlan,
  type NeedRow,
} from "../lib/dispatch";
import type {
  AddonLink,
  ContentSet,
  DispatchApplyReport,
  DispatchChanges,
  DispatchMode,
  DispatchState,
  DispatchTrip,
  LiveryEntry,
} from "../lib/types";
import { useCommunityStore } from "./useCommunityStore";
import { useToastStore } from "./useToastStore";

/** Qué se está revisando antes de confirmar. */
export type DispatchReview =
  /** `seqId`: la secuencia a lanzar al confirmar; `null` = sólo mirar. */
  | { kind: "flight"; flight: DispatchFlight | null; seqId: number | null }
  | { kind: "set"; setId: number };

export type SetsTab = "sets" | "restore" | "keep";

interface Store extends DispatchState {
  links: AddonLink[];
  /** (05/11) ICAO → país; vacío si no se conoce. */
  countries: Record<string, string>;
  ensureCountries: (codes: string[]) => Promise<void>;
  liveries: LiveryEntry[];
  loaded: boolean;
  /** Avión elegido por vuelo (sólo esta sesión: se pregunta en cada vuelo). */
  picks: Record<string, string>;
  review: DispatchReview | null;
  setsOpen: boolean;
  setsTab: SetsTab;
  /** MSFS se cerró con addons apagados: ofrecer restaurar. */
  restorePrompt: boolean;
  /** MSFS abierto al abrir la revisión. */
  simRunning: boolean;
  /** Ventana del viaje abierta, con el tipo del OFP para rellenarla. */
  tripEditor: { aircraft: string | null } | null;
  load: () => Promise<void>;
  setMode: (mode: DispatchMode, setId?: number | null) => Promise<void>;
  setUseAlternate: (on: boolean) => Promise<void>;
  setSceneryChoice: (icao: string, folder: string | null, ignored: boolean) => Promise<void>;
  openTrip: (aircraft: string | null) => void;
  closeTrip: () => void;
  saveTrip: (stops: string[], aircraft: string | null) => Promise<boolean>;
  clearTrip: () => Promise<void>;
  pick: (flightKey: string, value: string) => void;
  toggleKeep: (folder: string) => Promise<void>;
  saveSet: (name: string) => Promise<boolean>;
  updateSet: (id: number) => Promise<void>;
  renameSet: (id: number, name: string) => Promise<void>;
  deleteSet: (id: number) => Promise<void>;
  openReview: (review: DispatchReview) => void;
  closeReview: () => void;
  openSets: (tab?: SetsTab) => void;
  closeSets: () => void;
  applySet: (set: ContentSet) => Promise<void>;
  restore: (id?: number) => Promise<void>;
  dismissRestorePrompt: () => void;
}

const toastError = (title: string, e: unknown) =>
  useToastStore.getState().push({ kind: "error", title, message: String(e).replace(/^Error:\s*/, "") });

/** Refleja lo que cambió sin esperar a un escaneo. */
function markPackages(report: DispatchApplyReport) {
  const on = new Set(report.enabled);
  const off = new Set(report.disabled);
  useCommunityStore.setState((s) => ({
    packages: s.packages.map((p) =>
      on.has(p.folderName)
        ? { ...p, enabled: true }
        : off.has(p.folderName)
          ? { ...p, enabled: false }
          : p,
    ),
  }));
}

export const useDispatchStore = create<Store>((set, get) => ({
  mode: "just",
  setId: null,
  keep: [],
  sets: [],
  pendingRestore: null,
  useAlternate: true,
  trip: null,
  sceneryChoices: [],
  activeSet: null,
  tripEditor: null,
  links: [],
  countries: {},
  liveries: [],
  loaded: false,
  picks: {},
  review: null,
  setsOpen: false,
  setsTab: "sets",
  restorePrompt: false,
  simRunning: false,

  async ensureCountries(codes) {
    const known = get().countries;
    const unknown = codes.filter((code) => !(code in known));
    if (unknown.length === 0) return;
    try {
      const found = await api.lookupAirports(unknown);
      const next = { ...get().countries };
      for (const code of unknown) next[code] = found.find((a) => a.icao === code)?.isoCountry ?? "";
      set({ countries: next });
    } catch (e) {
      console.warn("[dispatch] países", e);
    }
  },

  async load() {
    try {
      const [state, links] = await Promise.all([api.dispatchState(), api.listAddonLinks()]);
      set({ ...state, links, loaded: true });
    } catch (e) {
      set({ loaded: true });
      console.warn("[dispatch] load", e);
    }
    // Aparte: la primera vez lee las liveries del disco y no debe frenar el resto.
    void api.dispatchLiveries().then((liveries) => set({ liveries })).catch(() => undefined);
  },

  async setMode(mode, setId = null) {
    set({ mode, setId });
    try {
      await api.dispatchSetMode(mode, setId);
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
    }
  },

  async setSceneryChoice(icao, folder, ignored) {
    try {
      set({ sceneryChoices: await api.dispatchSetScenery(icao, folder, ignored) });
    } catch (e) {
      toastError(t("dispatch.clash.error"), e);
    }
  },

  async setUseAlternate(on) {
    set({ useAlternate: on });
    try {
      await api.dispatchSetAlternate(on);
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
    }
  },

  openTrip: (aircraft) => set({ tripEditor: { aircraft } }),
  closeTrip: () => set({ tripEditor: null }),

  async saveTrip(stops, aircraft) {
    try {
      const trip = await api.dispatchSetTrip({ stops, aircraft });
      set({ trip, tripEditor: null });
      // Un viaje existe para encender addons: «Solo lanzar» no haría nada.
      const { mode } = get();
      if (mode === "just" || mode === "set") await get().setMode("add");
      return true;
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
      return false;
    }
  },

  async clearTrip() {
    try {
      await api.dispatchSetTrip(null);
      set({ trip: null, tripEditor: null });
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
    }
  },

  pick: (flightKey, value) => set((s) => ({ picks: { ...s.picks, [flightKey]: value } })),

  async toggleKeep(folder) {
    const keep = !get().keep.some((f) => f.toLowerCase() === folder.toLowerCase());
    try {
      set({ keep: await api.dispatchSetKeep(folder, keep) });
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
    }
  },

  async saveSet(name) {
    try {
      set(await api.contentSetSave(name));
      return true;
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
      return false;
    }
  },

  async updateSet(id) {
    try {
      set(await api.contentSetUpdate(id));
      useToastStore.getState().push({ kind: "success", title: t("dispatch.sets.updated"), ttlMs: 2600 });
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
    }
  },

  async renameSet(id, name) {
    try {
      set(await api.contentSetRename(id, name));
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
    }
  },

  async deleteSet(id) {
    try {
      set(await api.contentSetDelete(id));
    } catch (e) {
      toastError(t("dispatch.err.save"), e);
    }
  },

  openReview(review) {
    set({ review });
    void get().load();
    void api.launchSimInfo().then((info) => set({ simRunning: info.running })).catch(() => undefined);
  },
  closeReview: () => set({ review: null }),

  openSets(tab = "sets") {
    set({ setsOpen: true, setsTab: tab });
    void get().load();
  },
  closeSets: () => set({ setsOpen: false }),

  async applySet(target) {
    const plan = planSet(target, useCommunityStore.getState().packages, get().keep);
    const point = target.kind === "user" && plan.off.length > 0;
    try {
      const report = await api.dispatchApply({
        on: folders(plan.on),
        off: folders(plan.off),
        restoreName: point ? t("dispatch.restore.set_name", { name: target.name }) : null,
        clearPending: true,
        activeSet: target.kind === "user" ? target.id : null,
      });
      markPackages(report);
      set({ restorePrompt: false });
      await get().load();
      useToastStore.getState().push({
        kind: report.failed.length > 0 ? "error" : "success",
        title: t("dispatch.applied", { on: report.enabled.length, off: report.disabled.length }),
        message:
          report.failed.length > 0
            ? t("dispatch.failed", { n: report.failed.length })
            : get().simRunning
              ? t("dispatch.restart_sim")
              : undefined,
      });
    } catch (e) {
      toastError(t("dispatch.err.apply"), e);
    }
  },

  async restore(id) {
    const target = get().sets.find((s) => s.id === (id ?? get().pendingRestore));
    if (!target) return;
    const info = await api.launchSimInfo().catch(() => null);
    set({ simRunning: !!info?.running });
    await get().applySet(target);
  },

  dismissRestorePrompt: () => set({ restorePrompt: false }),
}));

export interface LaunchPlan extends DispatchPlan {
  /** Modo que se aplica de verdad. */
  mode: DispatchMode;
  set: ContentSet | null;
  /** Lo que se prepara: el viaje activo o, si no hay, el vuelo. */
  flight: DispatchFlight | null;
  trip: DispatchTrip | null;
  /** Modo de vuelo sin plan de vuelo: no hay nada que preparar. */
  noFlight: boolean;
  /** Hace falta pasar por la revisión antes de lanzar. */
  needsReview: boolean;
  /** La livery de la matrícula del OFP, sea cual sea el modo. */
  livery: NeedRow | null;
  changes: DispatchChanges | null;
}

/** (05/11) Países de los aeropuertos del vuelo, si algún enlace los pide. */
export function useFlightCountries(flight: DispatchFlight | null): Record<string, string> {
  const wanted = useDispatchStore((s) => s.links.some((l) => !!l.conditions?.country.length));
  const codes = flight ? flightAirports(flight).join(",") : "";
  useEffect(() => {
    if (wanted && codes) void useDispatchStore.getState().ensureCountries(codes.split(","));
  }, [wanted, codes]);
  return useDispatchStore((s) => s.countries);
}

/** (06/11) Conjunto activo y si lo encendido ya no coincide con él. Tras Enfocar
 *  no cuenta como cambio: es a propósito y hay una restauración pendiente. */
export function useActiveSet(): { set: ContentSet; changed: boolean } | null {
  const mode = useDispatchStore((s) => s.mode);
  const setId = useDispatchStore((s) => s.setId);
  const activeSet = useDispatchStore((s) => s.activeSet);
  const sets = useDispatchStore((s) => s.sets);
  const keep = useDispatchStore((s) => s.keep);
  const pending = useDispatchStore((s) => s.pendingRestore);
  const packages = useCommunityStore((s) => s.packages);
  return useMemo(() => {
    const id = mode === "set" && setId !== null ? setId : activeSet;
    const set = sets.find((s) => s.id === id && s.kind === "user");
    if (!set) return null;
    const plan = planSet(set, packages, keep);
    return { set, changed: pending === null && plan.on.length + plan.off.length > 0 };
  }, [mode, setId, activeSet, sets, keep, pending, packages]);
}

/** Lo que hará «Lanzar» con el modo elegido. */
export function useDispatchPlan(ofpFlight: DispatchFlight | null): LaunchPlan {
  const packages = useCommunityStore((s) => s.packages);
  const mode = useDispatchStore((s) => s.mode);
  const setId = useDispatchStore((s) => s.setId);
  const sets = useDispatchStore((s) => s.sets);
  const keep = useDispatchStore((s) => s.keep);
  const links = useDispatchStore((s) => s.links);
  const liveries = useDispatchStore((s) => s.liveries);
  const useAlternate = useDispatchStore((s) => s.useAlternate);
  const trip = useDispatchStore((s) => s.trip);
  const sceneryChoices = useDispatchStore((s) => s.sceneryChoices);
  const flight = useMemo(() => (trip ? tripFlight(trip) : ofpFlight), [trip, ofpFlight]);
  const pick = useDispatchStore((s) => (flight ? s.picks[flight.key] : undefined));
  const countries = useFlightCountries(flight);

  return useMemo(() => {
    const chosenSet = mode === "set" ? (sets.find((s) => s.id === setId) ?? null) : null;
    const effective: DispatchMode = mode === "set" && !chosenSet ? "just" : mode;
    const noFlight = !flight && (effective === "add" || effective === "focus");
    const forFlight = flight
      ? planFlight({
          mode: chosenSet ? "just" : effective,
          flight,
          packages,
          links,
          keep,
          liveries,
          useAlternate,
          pick,
          sceneryChoices,
          countries,
        })
      : EMPTY_PLAN;
    const plan = chosenSet ? planSet(chosenSet, packages, keep) : forFlight;
    const changed = plan.on.length + plan.off.length > 0;
    const route = flight ? `${flight.origin ?? "····"} → ${flight.destination ?? "····"}` : "";
    return {
      ...plan,
      mode: effective,
      set: chosenSet,
      flight,
      trip,
      noFlight,
      needsReview: plan.needsChoice || plan.off.length > 0,
      livery: forFlight.rows.find((r) => r.role === "livery") ?? null,
      changes: changed
        ? {
            on: folders(plan.on),
            off: folders(plan.off),
            restoreName: chosenSet
              ? t("dispatch.restore.set_name", { name: chosenSet.name })
              : t("dispatch.restore.focus_name", { route }),
            markPending: true,
            clearPending: true,
            activeSet: chosenSet?.id ?? null,
          }
        : null,
    };
  }, [flight, trip, packages, mode, setId, sets, keep, links, liveries, useAlternate, pick, sceneryChoices, countries]);
}

let wired = false;
/** Carga inicial + avisos de lo aplicado y del cierre del simulador. */
export function initDispatchSync() {
  if (wired) return;
  wired = true;
  void useDispatchStore.getState().load();
  if (!isTauri) return;
  void listen<DispatchApplyReport>("dispatch://applied", (e) => {
    markPackages(e.payload);
    useDispatchStore.setState({ pendingRestore: e.payload.pendingRestore });
    void useDispatchStore.getState().load();
  });
  void listen<number>("dispatch://sim-closed", () => {
    void useDispatchStore.getState().load();
    useDispatchStore.setState({ restorePrompt: true });
  });
}
