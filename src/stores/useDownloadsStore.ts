import { create } from "zustand";
import type {
  CommunityInfo,
  DownloadJob,
  DownloadMethod,
  InstalledAddon,
} from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useCommunityStore } from "./useCommunityStore";
import { useToastStore } from "./useToastStore";

/** Pestañas dentro del panel lateral. La de descargas activas es la
 *  que ya existía; «installed» es la nueva pestaña con el historial
 *  persistente leído desde `installed_addons`. */
export type PanelTab = "active" | "installed";

interface DownloadsState {
  /** Jobs indexed by id → keeps reconciliation O(1) and gives stable iteration order. */
  jobs: Record<string, DownloadJob>;
  /** Lista persistente leída de la DB. Se refresca en bootstrap y
   *  después de cada cambio (instalar manualmente, olvidar fila…). */
  installed: InstalledAddon[];
  isPanelOpen: boolean;
  panelTab: PanelTab;
  community: CommunityInfo | null;
  /** Estado del flujo manual «Instalar desde archivo…»: lo
   *  exponemos para que el botón pueda mostrar un spinner sin que
   *  cada componente tenga que llevar su propio estado local. */
  manualInstallBusy: boolean;
  manualInstallError: string | null;

  setPanelOpen: (open: boolean) => void;
  togglePanel: () => void;
  setPanelTab: (tab: PanelTab) => void;
  /** Atajo: abre el panel y selecciona una pestaña concreta. */
  openPanelAt: (tab: PanelTab) => void;

  bootstrap: () => Promise<void>;
  reconcile: (job: DownloadJob) => void;

  start: (input: {
    addonId: string;
    addonTitle: string;
    source: string;
    method: DownloadMethod;
  }) => Promise<void>;
  cancel: (id: string) => Promise<void>;
  pause: (id: string) => Promise<void>;
  resume: (id: string) => Promise<void>;
  clear: (id: string) => Promise<void>;

  /** Refresca la lista persistente desde el backend. */
  refreshInstalled: () => Promise<void>;
  /** Abre el selector de archivos y dispara la instalación. Devuelve
   *  `true` si se instaló algo, `false` si el usuario canceló. */
  installFromFile: () => Promise<boolean>;
  forgetInstalled: (id: string) => Promise<void>;
}

export const useDownloadsStore = create<DownloadsState>((set, get) => ({
  jobs: {},
  installed: [],
  isPanelOpen: false,
  panelTab: "active",
  community: null,
  manualInstallBusy: false,
  manualInstallError: null,

  setPanelOpen: (isPanelOpen) => set({ isPanelOpen }),
  togglePanel: () => set((s) => ({ isPanelOpen: !s.isPanelOpen })),
  setPanelTab: (panelTab) => set({ panelTab }),
  openPanelAt: (panelTab) => set({ panelTab, isPanelOpen: true }),

  async bootstrap() {
    // Force the panel closed on every fresh mount. Zustand preserves
    // store state across Vite HMR by default, which can otherwise leave
    // the panel stuck open after a code reload and make the scrim hide
    // the whole UI — that was a real bug seen during development.
    set({ isPanelOpen: false, panelTab: "active" });

    // Initial snapshot + live updates. The reconciler is "last write
    // wins" so the order of listen-vs-list doesn't matter.
    await api.onDownloadUpdate((job) => get().reconcile(job));
    const initial = await api.listDownloads();
    const map: Record<string, DownloadJob> = {};
    for (const j of initial) map[j.id] = j;
    set({ jobs: map });

    try {
      const community = await api.communityFolder();
      set({ community });
    } catch {
      set({ community: null });
    }

    // Cargar el historial persistente. Si falla, no es bloqueante:
    // la pestaña «Instalados» mostrará una lista vacía y un botón
    // para reintentar.
    try {
      const installed = await api.listInstalled();
      set({ installed });
    } catch (e) {
      console.warn("listInstalled failed:", e);
      set({ installed: [] });
    }
  },

  reconcile(job) {
    // Backend uses the magic message "__removed__" to signal deletion
    // because Tauri events don't carry a separate "removed" channel —
    // keeping one event stream is simpler than two.
    if (job.message === "__removed__") {
      set((s) => {
        const next = { ...s.jobs };
        delete next[job.id];
        return { jobs: next };
      });
      return;
    }
    const prev = get().jobs[job.id];
    set((s) => ({ jobs: { ...s.jobs, [job.id]: job } }));

    // Toasts en transiciones a estados terminales. Comprobamos el
    // delta `prev?.phase` para no spamear si llegan varios eventos
    // con el mismo estado (race entre invoke-return y onDownloadUpdate).
    if (job.phase === "completed" && prev?.phase !== "completed") {
      useToastStore.getState().push({
        kind: "success",
        title: t("toast.download.installed"),
        message: job.addonTitle,
        onClick: () => useDownloadsStore.getState().openPanelAt("installed"),
      });
      get()
        .refreshInstalled()
        .catch((e) => console.warn("refreshInstalled failed:", e));
      useCommunityStore
        .getState()
        .rescan()
        .catch((e) => console.warn("community rescan failed:", e));
    } else if (job.phase === "error" && prev?.phase !== "error") {
      useToastStore.getState().push({
        kind: "error",
        title: t("toast.download.failed"),
        message: job.error ?? job.addonTitle,
        onClick: () => useDownloadsStore.getState().openPanelAt("active"),
        ttlMs: null, // los errores se quedan hasta que el usuario los cierra
      });
    }
  },

  async start({ addonId, addonTitle, source, method }) {
    // Intentionally do NOT auto-open the panel here. The button in the
    // header has a live badge count, and surprising the user with a
    // full-screen scrim on every click is worse UX than letting them
    // open the panel themselves when they care.
    try {
      const job = await api.startDownload({ addonId, addonTitle, source, method });
      // Ensure the store has the initial state even if the event for
      // "queued" beat us (race between invoke-return and event-emit).
      get().reconcile(job);
      // Toast inmediato — confirma al usuario que el click funcionó
      // sin obligarle a abrir el panel de descargas.
      useToastStore.getState().push({
        kind: "info",
        title: t("toast.download.started"),
        message: `${addonTitle} (${method.kind})`,
        onClick: () => useDownloadsStore.getState().openPanelAt("active"),
      });
    } catch (e) {
      useToastStore.getState().push({
        kind: "error",
        title: t("toast.download.start_error"),
        message: String(e),
      });
      throw e;
    }
  },

  async cancel(id) {
    await api.cancelDownload(id);
  },

  async pause(id) {
    await api.pauseDownload(id);
  },

  async resume(id) {
    await api.resumeDownload(id);
  },

  async clear(id) {
    await api.clearDownload(id);
  },

  async refreshInstalled() {
    const installed = await api.listInstalled();
    set({ installed });
  },

  async installFromFile() {
    set({ manualInstallBusy: true, manualInstallError: null });
    try {
      const result = await api.installFromFile();
      if (!result) {
        // El usuario canceló el picker — no es un error.
        return false;
      }
      // Tras instalar, refrescamos el historial y abrimos el panel
      // en la pestaña «Instalados» para que el usuario vea de
      // inmediato el resultado de su acción.
      await get().refreshInstalled();
      get().openPanelAt("installed");
      return true;
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      set({ manualInstallError: msg });
      return false;
    } finally {
      set({ manualInstallBusy: false });
    }
  },

  async forgetInstalled(id) {
    await api.forgetInstall(id);
    // Optimismo local: quitamos la fila sin esperar a un nuevo fetch.
    set((s) => ({ installed: s.installed.filter((x) => x.id !== id) }));
  },
}));

/**
 * Derive a newest-first jobs array from the store's jobs map.
 *
 * Why a standalone helper instead of a zustand selector: passing a
 * selector that returns `Object.values().sort()` directly to
 * `useDownloadsStore` produces a new array reference on every call,
 * which under React 18 + StrictMode trips the "result of getSnapshot
 * should be cached" check and can stall rendering. Components should
 * subscribe to `state.jobs` (stable ref) and call this helper inside
 * a `useMemo` keyed on the jobs map.
 */
export function toJobsList(jobs: Record<string, DownloadJob>): DownloadJob[] {
  return Object.values(jobs).sort((a, b) => b.createdAt.localeCompare(a.createdAt));
}
