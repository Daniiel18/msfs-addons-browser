import { create } from "zustand";
import type {
  AvailableUpdate,
  CommunityPackage,
  ToggleReport,
} from "../lib/types";
import { api } from "../lib/tauri";

/**
 * Estado compartido entre la vista de mapa y la campana de
 * notificaciones del header. Mantiene en memoria:
 *   · El inventario de paquetes en Community (lo que el scanner
 *     leyó la última vez).
 *   · La lista de updates detectados (ya filtrada de "dismissed").
 *   · La cola del wizard "Actualizar todo".
 *   · Banderas de carga para refrescos manuales.
 *
 * El bootstrap se llama una vez al montar `App` y dispara el
 * scan + el refresh activo en background — no bloquea la UI.
 */
interface CommunityState {
  packages: CommunityPackage[];
  updates: AvailableUpdate[];

  /** Paquete enfocado en el mapa: la cámara se centra en él y se
   *  resalta en la sidebar. **No** abre el modal — esa es una
   *  acción separada (`detailsFor`) para que el usuario pueda
   *  apreciar el zoom sin un overlay encima. */
  focused: string | null;

  /** Paquete cuyo modal de detalle está abierto. Independiente de
   *  `focused`: el usuario puede tener algo enfocado en el mapa
   *  sin que el modal esté visible. */
  detailsFor: string | null;

  /** Cola del wizard "Actualizar todo". Cuando es no-vacía, el
   *  componente `UpdateWizard` se monta sobre el resto de la app
   *  y muestra el primer item. Al elegir método (o saltar), shift
   *  + sigue; cuando queda vacía el wizard se cierra. */
  updateQueue: AvailableUpdate[];

  scanning: boolean;
  refreshing: boolean;
  lastScanError: string | null;
  lastRefreshError: string | null;

  bootstrap: () => Promise<void>;
  /** Sólo el escaneo del FS + sync a DB — sin tocar la red.
   *  Lo expone aparte para que el splash pueda mostrarlo como
   *  paso individual del bootstrap. */
  scanFromFS: () => Promise<void>;
  /** Sólo el refresh activo contra fuentes — depende de un scan
   *  reciente. Lo expone aparte para que el splash lo cuente como
   *  un paso visible. */
  refreshUpdatesActive: () => Promise<void>;
  rescan: () => Promise<void>;
  /** Refresh manual — limpia primero todas las dismissed para que
   *  el usuario siempre vea lo que sigue pendiente. */
  refreshUpdates: () => Promise<void>;
  setFocused: (folderName: string | null) => void;
  openDetails: (folderName: string | null) => void;

  /** (v4.25.0) Enciende/apaga UN paquete con cascada por el Link Map
   *  (los enlazados siguen al padre). Actualiza el estado en memoria
   *  para todos los folders que el backend reporta como cambiados. */
  setEnabled: (folderName: string, enabled: boolean) => Promise<ToggleReport>;
  /** (v4.25.0) Toggle masivo — Enable All / Disable All. */
  setManyEnabled: (
    folderNames: string[],
    enabled: boolean,
  ) => Promise<ToggleReport>;

  // Dismiss API (delegan a backend)
  dismissUpdate: (folderName: string) => Promise<void>;
  dismissAll: () => Promise<void>;

  // Wizard API
  startUpdateAll: () => void;
  shiftUpdateQueue: () => void;
  cancelUpdateQueue: () => void;
}

export const useCommunityStore = create<CommunityState>((set, get) => ({
  packages: [],
  updates: [],
  focused: null,
  detailsFor: null,
  updateQueue: [],
  scanning: false,
  refreshing: false,
  lastScanError: null,
  lastRefreshError: null,

  /**
   * Bootstrap completo: escaneo FS + refresh activo contra fuentes.
   * Awaiteado por el splash para que el usuario no vea «cargando»
   * después de entrar a la app.
   */
  async bootstrap() {
    try {
      await get().scanFromFS();
      await get().refreshUpdatesActive();
    } catch (e) {
      console.warn("communityStore.bootstrap failed:", e);
    }
  },

  /** Sólo el scan del FS — barato (lee disco, no toca red). */
  async scanFromFS() {
    set({ scanning: true, lastScanError: null });
    try {
      await api.scanCommunity();
      const pkgs = await api.listCommunityPackages();
      set({ packages: pkgs });
    } catch (e) {
      set({ lastScanError: String(e) });
      throw e;
    } finally {
      set({ scanning: false });
    }
  },

  /** Refresh activo contra fuentes (queries HTTP cacheadas 6h). */
  async refreshUpdatesActive() {
    set({ refreshing: true, lastRefreshError: null });
    try {
      await api.refreshUpdatesForInstalled();
      const updates = await api.listAvailableUpdates();
      set({ updates });
    } catch (e) {
      set({ lastRefreshError: String(e) });
      throw e;
    } finally {
      set({ refreshing: false });
    }
  },

  async rescan() {
    if (get().scanning) return;
    set({ scanning: true, lastScanError: null });
    try {
      await api.scanCommunity();
      const [pkgs, updates] = await Promise.all([
        api.listCommunityPackages(),
        api.listAvailableUpdates(),
      ]);
      set({ packages: pkgs, updates });
    } catch (e) {
      set({ lastScanError: String(e) });
    } finally {
      set({ scanning: false });
    }
  },

  async refreshUpdates() {
    if (get().refreshing) return;
    set({ refreshing: true, lastRefreshError: null });
    try {
      // Reset de las descartadas — el usuario pulsó refresh, así
      // que quiere ver TODO lo pendiente, no quedarse con el filtro
      // de la sesión anterior.
      await api.clearDismissedUpdates().catch(() => {});
      await api.refreshUpdatesForInstalled();
      const updates = await api.listAvailableUpdates();
      set({ updates });
    } catch (e) {
      set({ lastRefreshError: String(e) });
    } finally {
      set({ refreshing: false });
    }
  },

  setFocused: (folderName) => set({ focused: folderName }),
  openDetails: (folderName) => set({ detailsFor: folderName }),

  async setEnabled(folderName, enabled) {
    // cascade=true SIEMPRE desde la UI: encender un aircraft enciende
    // sus liveries/sonidos enlazados; apagarlo los apaga. El backend
    // hace BFS cycle-safe y devuelve la lista real de cambiados.
    const report = await api.setPackageEnabled(folderName, enabled, true);
    const changed = new Set(report.changed);
    set((s) => ({
      packages: s.packages.map((p) =>
        changed.has(p.folderName) ? { ...p, enabled } : p,
      ),
    }));
    return report;
  },

  async setManyEnabled(folderNames, enabled) {
    const report = await api.setPackagesEnabled(folderNames, enabled);
    const changed = new Set(report.changed);
    set((s) => ({
      packages: s.packages.map((p) =>
        changed.has(p.folderName) ? { ...p, enabled } : p,
      ),
    }));
    return report;
  },

  async dismissUpdate(folderName) {
    // Optimista: lo quitamos de la lista en memoria al instante.
    set((s) => ({ updates: s.updates.filter((u) => u.folderName !== folderName) }));
    try {
      await api.dismissUpdate(folderName);
    } catch (e) {
      console.warn("dismissUpdate falló:", e);
      // Recargar listado — si la persistencia falló queremos que
      // la UI vuelva a reflejar el estado real.
      const updates = await api.listAvailableUpdates().catch(() => get().updates);
      set({ updates });
    }
  },

  async dismissAll() {
    set({ updates: [] });
    try {
      await api.dismissAllUpdates();
    } catch (e) {
      console.warn("dismissAllUpdates falló:", e);
      const updates = await api.listAvailableUpdates().catch(() => []);
      set({ updates });
    }
  },

  startUpdateAll() {
    // Snapshot del listado actual al pulsar el botón. Si llegan
    // nuevas updates mientras el wizard corre, no las metemos en
    // la cola para no marear al usuario; las verá la siguiente vez.
    set({ updateQueue: [...get().updates] });
  },
  shiftUpdateQueue() {
    set((s) => ({ updateQueue: s.updateQueue.slice(1) }));
  },
  cancelUpdateQueue() {
    set({ updateQueue: [] });
  },
}));
