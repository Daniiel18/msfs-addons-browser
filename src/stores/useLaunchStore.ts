import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api, isTauri } from "../lib/tauri";
import { t } from "../lib/i18n";
import { launchError } from "../lib/launchFlow";
import type {
  DispatchChanges,
  LaunchDetectedApp,
  LaunchRun,
  LaunchScriptPermission,
  LaunchSequence,
  LaunchSimInfo,
  LaunchStep,
} from "../lib/types";
import { useToastStore } from "./useToastStore";

/**
 * (02/11) Etapa 6 — secuencias de arranque: lista, apps del PC, el simulador
 * y la secuencia en marcha (`launch://progress`).
 */
interface LaunchState {
  sequences: LaunchSequence[];
  apps: LaunchDetectedApp[];
  sim: LaunchSimInfo | null;
  run: LaunchRun | null;
  loaded: boolean;
  editorOpen: boolean;
  /** Secuencia a mostrar al abrir el editor. */
  editorFocus: number | null;
  /** Elegida a mano para un tipo de avión (sólo esta sesión). */
  chosen: { aircraft: string; id: number } | null;
  /** (07/11) Scripts que piden permiso y qué hacer al darlo. */
  scriptAsk: { scripts: LaunchScriptPermission[]; retry: () => void } | null;
  allowScripts: () => Promise<void>;
  denyScripts: () => void;
  load: () => Promise<void>;
  loadApps: () => Promise<void>;
  /** Guarda y devuelve el id guardado (null si falló). */
  save: (seq: LaunchSequence) => Promise<number | null>;
  remove: (id: number) => Promise<void>;
  /** `changes`: addons ya confirmados, que se aplican antes de abrir nada. */
  launch: (id: number, testAll?: boolean, changes?: DispatchChanges) => Promise<void>;
  cancel: () => Promise<void>;
  skipWait: () => Promise<void>;
  testStep: (step: LaunchStep) => Promise<void>;
  openEditor: (id?: number | null) => void;
  closeEditor: () => void;
  choose: (aircraft: string, id: number) => void;
  dismissRun: () => void;
}

const toastError = (title: string, e: unknown) =>
  useToastStore.getState().push({ kind: "error", title, message: launchError(String(e).replace(/^Error:\s*/, "")) });

/** Un script sin permiso: se pregunta y, al darlo, se repite lo que pediste. */
async function askScripts(e: unknown, steps: LaunchStep[], retry: () => void): Promise<boolean> {
  if (!String(e).includes("script_permission")) return false;
  const scripts = await api.launchScriptsPending(steps).catch(() => []);
  if (scripts.length === 0) return false;
  useLaunchStore.setState({ scriptAsk: { scripts, retry } });
  return true;
}

export const useLaunchStore = create<LaunchState>((set, get) => ({
  sequences: [],
  apps: [],
  sim: null,
  run: null,
  loaded: false,
  editorOpen: false,
  editorFocus: null,
  chosen: null,
  scriptAsk: null,

  async allowScripts() {
    const ask = get().scriptAsk;
    if (!ask) return;
    try {
      await api.launchScriptsApprove(ask.scripts.map((s) => s.hash));
      set({ scriptAsk: null });
      ask.retry();
    } catch (e) {
      set({ scriptAsk: null });
      toastError(t("launch.err.run"), e);
    }
  },
  denyScripts: () => set({ scriptAsk: null }),

  async load() {
    try {
      const [sequences, sim] = await Promise.all([api.launchSequences(), api.launchSimInfo()]);
      set({ sequences, sim, loaded: true });
    } catch (e) {
      set({ loaded: true });
      console.warn("[launch] load", e);
    }
  },

  async loadApps() {
    try {
      set({ apps: await api.launchDetectedApps() });
    } catch (e) {
      console.warn("[launch] apps", e);
    }
  },

  async save(seq) {
    try {
      const before = new Set(get().sequences.map((s) => s.id));
      const sequences = await api.launchSequenceSave(seq);
      set({ sequences });
      if (seq.id > 0) return seq.id;
      return sequences.find((s) => !before.has(s.id))?.id ?? null;
    } catch (e) {
      toastError(t("launch.err.save"), e);
      return null;
    }
  },

  async remove(id) {
    try {
      set({ sequences: await api.launchSequenceDelete(id) });
    } catch (e) {
      toastError(t("launch.err.delete"), e);
    }
  },

  async launch(id, testAll = false, changes) {
    try {
      const run = await api.launchRun(id, testAll, changes);
      // Los avisos de progreso pueden llegar antes que esta respuesta.
      if (get().run?.runId !== run.runId) set({ run });
    } catch (e) {
      const steps = get().sequences.find((s) => s.id === id)?.steps ?? [];
      if (await askScripts(e, steps, () => void get().launch(id, testAll, changes))) return;
      toastError(t("launch.err.run"), e);
    }
  },

  async cancel() {
    await api.launchCancel().catch(() => undefined);
  },

  async skipWait() {
    await api.launchSkipWait().catch(() => undefined);
  },

  async testStep(step) {
    try {
      const r = await api.launchTestStep(step);
      useToastStore.getState().push({
        kind: "success",
        title: t(r === "already" ? "launch.test.already" : "launch.test.opened", { name: step.name }),
        ttlMs: 2600,
      });
    } catch (e) {
      if (await askScripts(e, [step], () => void get().testStep(step))) return;
      toastError(t("launch.test.failed", { name: step.name }), e);
    }
  },

  openEditor(id = null) {
    set({ editorOpen: true, editorFocus: id });
    if (get().apps.length === 0) void get().loadApps();
    void get().load();
  },
  closeEditor: () => set({ editorOpen: false }),
  choose: (aircraft, id) => set({ chosen: { aircraft, id } }),
  dismissRun: () => set({ run: null }),
}));

let wired = false;
/** Carga inicial + escucha de la secuencia en marcha (una sola vez). */
export function initLaunchSync() {
  if (wired) return;
  wired = true;
  void useLaunchStore.getState().load();
  if (!isTauri) return;
  void api.launchStatus().then((run) => run?.state === "running" && useLaunchStore.setState({ run }));
  void listen<LaunchRun>("launch://progress", (e) => {
    const run = e.payload;
    // Una que el usuario ya cerró no vuelve a aparecer por un aviso tardío.
    const cur = useLaunchStore.getState().run;
    if (!cur && run.state !== "running") return;
    useLaunchStore.setState({ run });
  });
  // Al salir del simulador se cerraron las apps marcadas. (05/11) Solo cuenta
  // lo que se cerró de verdad; lo que siguió abierto tiene su propio aviso.
  void listen<{ closed: string[]; open: string[] }>("launch://closed", (e) => {
    const { closed = [], open = [] } = e.payload ?? {};
    if (closed.length > 0) {
      useToastStore.getState().push({
        kind: "success",
        title: t("launch.closed.title", { n: String(closed.length) }),
        message: closed.join(" · "),
      });
    }
    if (open.length > 0) {
      useToastStore.getState().push({
        kind: "error",
        title: t("launch.closed.open_title", { n: String(open.length) }),
        message: `${open.join(" · ")}. ${t("launch.closed.open_hint", { admin: t("launch.open.admin") })}`,
        ttlMs: null,
      });
    }
  });
}
