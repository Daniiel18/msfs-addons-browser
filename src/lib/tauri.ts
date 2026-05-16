import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
// `open` (file picker) viene del plugin-dialog. Lo importamos
// dinámicamente cuando lo necesitamos para no romper el modo demo
// (donde el bundle del plugin no está disponible al ejecutarse en un
// navegador puro).
import type {
  Addon,
  AddonOnMap,
  AppSettings,
  OcDetectionReport,
  AvailableUpdate,
  BackupResult,
  BrowsePage,
  Changelog,
  CommunityInfo,
  CommunityPackage,
  DashboardStats,
  DownloadJob,
  ExportFormat,
  ExportResult,
  FlightLogEntry,
  FlightStatus,
  DownloadMethod,
  GsxProfile,
  InstallResult,
  InstalledAddon,
  RefreshSummary,
  ScanReport,
  SimBriefFlight,
  SimBriefRefreshResult,
  SourceDescriptor,
  UpdateDiagnostic,
  UpdateInfo,
} from "./types";

/** True when we're running inside the Tauri webview (vs. a plain browser). */
export const isTauri =
  typeof window !== "undefined" &&
  Object.prototype.hasOwnProperty.call(window, "__TAURI_INTERNALS__");

interface StartDownloadInput {
  addonId: string;
  addonTitle: string;
  source: string;
  method: DownloadMethod;
}

interface Api {
  listSources: () => Promise<SourceDescriptor[]>;
  search: (query: string, sourceId: string) => Promise<Addon[]>;
  /** Lista paginada del catálogo (sin query). Page 1-based. */
  browseSource: (sourceId: string, page: number) => Promise<BrowsePage>;
  openExternal: (url: string) => Promise<void>;
  /** Abre un path local con la app por defecto del SO (Explorer en
   *  Windows, Finder en macOS). Lo usamos para «Abrir carpeta» desde
   *  la pestaña «Instalados». */
  openLocalPath: (path: string) => Promise<void>;

  // Downloads
  listDownloads: () => Promise<DownloadJob[]>;
  startDownload: (input: StartDownloadInput) => Promise<DownloadJob>;
  cancelDownload: (id: string) => Promise<void>;
  pauseDownload: (id: string) => Promise<void>;
  resumeDownload: (id: string) => Promise<void>;
  clearDownload: (id: string) => Promise<void>;
  onDownloadUpdate: (cb: (job: DownloadJob) => void) => Promise<UnlistenFn>;

  // Install / environment
  communityFolder: () => Promise<CommunityInfo | null>;
  /** Abre el selector nativo y, si el usuario elige un .zip/.rar/.7z,
   *  lanza la instalación. Devuelve `null` si el usuario canceló el
   *  picker — la UI distingue ese caso del error real. */
  installFromFile: () => Promise<InstallResult | null>;
  /** Lista persistente de paquetes instalados (torrent + manuales). */
  listInstalled: () => Promise<InstalledAddon[]>;
  /** Quita una fila del historial. No toca el disco. */
  forgetInstall: (id: string) => Promise<void>;
  /** Diagnóstico: dónde está/debería estar `PMDG Operations Center.exe`. */
  diagnosePmdgOc: () => Promise<OcDetectionReport>;
  /** Diálogo nativo de elección de archivo. Filtros: `{name, extensions}`. */
  pickFilePath: (filters?: { name: string; extensions: string[] }[]) => Promise<string | null>;

  // GSX
  /** Devuelve los perfiles GSX Pro publicados para `icao` (puede estar
   *  vacío). El backend cachea ~24h en SQLite, así que llamarlo por
   *  cada resultado de búsqueda es barato después del primer barrido. */
  gsxLookup: (icao: string) => Promise<GsxProfile[]>;

  // Actualizaciones
  /** Verifica contra GitHub Releases si hay una versión mayor que la
   *  instalada. Devuelve `null` cuando estamos al día o cuando la
   *  consulta falló (offline, GitHub caído, etc.). */
  checkForUpdate: () => Promise<UpdateInfo | null>;
  /** Baja el instalador del asset indicado a `%TEMP%`, lo lanza en
   *  modo silent (`/S` NSIS · `/quiet` MSI), y cierra la app actual
   *  para que el installer pueda reemplazar los archivos en disco.
   *  Si todo va bien la app no retorna — el `exit(0)` la mata. Si
   *  falla la descarga o el spawn devuelve un error. */
  installUpdate: (assetUrl: string) => Promise<void>;
  /** Listener del progreso de descarga del installer. Se suscribe
   *  durante `installUpdate` y se llama cada chunk con bytes
   *  descargados + total (puede ser `null` si el servidor no envió
   *  Content-Length). Devuelve unsubscribe. */
  onUpdateProgress: (
    cb: (p: { downloadedBytes: number; totalBytes: number | null }) => void,
  ) => Promise<() => void>;

  // Mapa
  /** Devuelve los addons del catálogo local con coords resolvibles.
   *  Vacío si la tabla de aeropuertos aún no se ha sincronizado o
   *  ningún addon del catálogo tiene ICAO conocido. */
  listAddonsOnMap: () => Promise<AddonOnMap[]>;
  /** Fuerza un refresco del dataset de aeropuertos. Devuelve cuántas
   *  filas quedaron en la tabla tras el fetch. */
  refreshAirportsDataset: () => Promise<number>;

  // Community
  /** Escanea el folder Community + sincroniza con DB. */
  scanCommunity: () => Promise<ScanReport>;
  /** Lista todos los paquetes en Community (ya escaneados). */
  listCommunityPackages: () => Promise<CommunityPackage[]>;
  /** Devuelve las updates conocidas (compara cache vs instalado). */
  listAvailableUpdates: () => Promise<AvailableUpdate[]>;
  /** Hace barrido activo: queries por cada ICAO instalado. */
  refreshUpdatesForInstalled: () => Promise<RefreshSummary>;
  /** Borra el directorio del paquete en Community + filas de DB. */
  uninstallCommunityPackage: (folderName: string) => Promise<void>;
  /** Diagnóstico de update — devuelve el estado completo de la
   *  cadena (paquete, airport match, catalog entries, cache,
   *  bloqueador). Lo invoca el botón "Diagnosticar" del modal. */
  diagnoseUpdateForPackage: (folderName: string) => Promise<UpdateDiagnostic>;

  /** Data URL `data:image/<mime>;base64,...` con el thumbnail del
   *  paquete, o `null` si no encontró ninguno. El backend lee y
   *  encodea el archivo — evita el problema de scope del asset
   *  protocol con paths arbitrarios fuera de la carpeta de la app. */
  packageThumbnail: (folderName: string) => Promise<string | null>;

  /** Scrape el changelog desde la página de detalle del addon. */
  fetchChangelog: (pageUrl: string) => Promise<Changelog>;

  // SimBrief
  getSimbriefPilotId: () => Promise<string | null>;
  setSimbriefPilotId: (pilotId: string) => Promise<void>;
  refreshSimbrief: () => Promise<SimBriefRefreshResult>;
  listSimbriefFlights: () => Promise<SimBriefFlight[]>;
  deleteSimbriefFlight: (ofpId: string) => Promise<void>;

  // Dismiss de updates
  dismissUpdate: (folderName: string) => Promise<void>;
  dismissAllUpdates: () => Promise<void>;
  clearDismissedUpdates: () => Promise<void>;

  // Dashboard
  getDashboardStats: () => Promise<DashboardStats>;

  // Settings
  getAppSettings: () => Promise<AppSettings>;
  setAppSetting: (key: string, value: string) => Promise<void>;
  setAutostart: (enabled: boolean) => Promise<boolean>;
  clearCaches: () => Promise<number>;
  resetSettings: () => Promise<number>;
  /** Comprime la carpeta Community en un .zip. `destPath` puede ser
   *  un directorio (la app pone el nombre con timestamp) o un .zip
   *  concreto. */
  backupCommunity: (destPath: string) => Promise<BackupResult>;
  /** Exporta el inventario a CSV / TXT / JSON. */
  exportAddons: (destPath: string, format: ExportFormat) => Promise<ExportResult>;
  /** Diálogo nativo «save as» — devuelve la ruta elegida o null si
   *  el usuario canceló. */
  pickSavePath: (defaultName: string, filters?: { name: string; extensions: string[] }[]) => Promise<string | null>;
  /** Diálogo nativo «select folder». */
  pickFolderPath: () => Promise<string | null>;

  // Flight log (SimConnect)
  listFlightLog: () => Promise<FlightLogEntry[]>;
  deleteFlightLogEntry: (id: number) => Promise<void>;
  /** Inserta un vuelo de prueba EBBR→LEMD para validar la UI sin
   *  necesidad de tener MSFS corriendo. No usar en producción. */
  debugSeedFlightLog: () => Promise<number>;
  /** Suscribe un callback a cambios en el flight_log (emit del
   *  watcher cuando empieza o termina un vuelo). */
  onFlightLogChange: (cb: () => void) => Promise<UnlistenFn>;
  /** Estado actual del watcher de vuelo (MSFS proceso + OFP). */
  getFlightStatus: () => Promise<FlightStatus>;
  /** Suscribe un callback a cambios en el estado de vuelo —
   *  el watcher emite `flight://current` cuando cambia. */
  onFlightStatus: (cb: (status: FlightStatus) => void) => Promise<UnlistenFn>;
}

const realApi: Api = {
  listSources: () => invoke<SourceDescriptor[]>("list_sources"),
  search: (query, sourceId) => invoke<Addon[]>("search", { query, sourceId }),
  browseSource: (sourceId, page) =>
    invoke<BrowsePage>("browse_source", { sourceId, page }),
  // `tauri-plugin-opener` reemplaza al viejo `plugin-shell` para
  // abrir paths y URLs. Diferencia clave: `openPath` invoca al
  // shell del sistema (Explorer en Windows, Finder en macOS) sin
  // exigir que el path esté declarado en `tauri.conf.json` —
  // `plugin-shell.open` rechazaba paths que no machearan el scope
  // y por eso "Abrir carpeta" fallaba en silencio para los addons
  // instalados.
  openExternal: (url) => openUrl(url),
  openLocalPath: (path) => openPath(path),

  listDownloads: () => invoke<DownloadJob[]>("list_downloads"),
  startDownload: ({ addonId, addonTitle, source, method }) =>
    invoke<DownloadJob>("start_download", {
      addonId,
      addonTitle,
      source,
      method,
    }),
  cancelDownload: (id) => invoke<void>("cancel_download", { id }),
  pauseDownload: (id) => invoke<void>("pause_download", { id }),
  resumeDownload: (id) => invoke<void>("resume_download", { id }),
  clearDownload: (id) => invoke<void>("clear_download", { id }),
  onDownloadUpdate: (cb) =>
    listen<DownloadJob>("download://update", (event) => cb(event.payload)),

  communityFolder: () => invoke<CommunityInfo | null>("community_folder"),
  async installFromFile() {
    // Import dinámico: el plugin no existe en modo demo (browser puro)
    // y no queremos que ese build falle por una dep ausente.
    const { open } = await import("@tauri-apps/plugin-dialog");
    const selected = await open({
      multiple: false,
      directory: false,
      title: "Selecciona un archivo .zip/.rar/.7z",
      filters: [
        { name: "Archivos comprimidos", extensions: ["zip", "rar", "7z"] },
      ],
    });
    if (selected === null) return null;
    // El plugin devuelve `string | string[]` — `multiple:false` ya
    // garantiza el primer caso, pero aplastamos por seguridad.
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) return null;
    return invoke<InstallResult>("install_archive", {
      archivePath: path,
      communityPath: null,
    });
  },
  listInstalled: () => invoke<InstalledAddon[]>("list_installed"),
  forgetInstall: (id) => invoke<void>("forget_install", { id }),
  diagnosePmdgOc: () => invoke<OcDetectionReport>("diagnose_pmdg_oc"),
  async pickFilePath(filters) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({ multiple: false, directory: false, filters });
    if (result === null) return null;
    return typeof result === "string" ? result : null;
  },

  gsxLookup: (icao) => invoke<GsxProfile[]>("gsx_lookup", { icao }),

  checkForUpdate: () => invoke<UpdateInfo | null>("check_for_update"),
  installUpdate: (assetUrl) =>
    invoke<void>("install_update", { assetUrl }),
  onUpdateProgress: async (cb) => {
    // Lazy-import del evento — los entornos non-Tauri (vitest, demo
    // mode) no traen `@tauri-apps/api/event` con un canal real.
    const { listen } = await import("@tauri-apps/api/event");
    const unlisten = await listen<{
      downloadedBytes: number;
      totalBytes: number | null;
    }>("updater://progress", (e) => cb(e.payload));
    return () => unlisten();
  },

  listAddonsOnMap: () => invoke<AddonOnMap[]>("list_addons_on_map"),
  refreshAirportsDataset: () => invoke<number>("refresh_airports_dataset"),

  scanCommunity: () => invoke<ScanReport>("scan_community", { communityPath: null }),
  listCommunityPackages: () =>
    invoke<CommunityPackage[]>("list_community_packages"),
  listAvailableUpdates: () => invoke<AvailableUpdate[]>("list_available_updates"),
  refreshUpdatesForInstalled: () =>
    invoke<RefreshSummary>("refresh_updates_for_installed"),
  uninstallCommunityPackage: (folderName) =>
    invoke<void>("uninstall_community_package", { folderName }),
  diagnoseUpdateForPackage: (folderName) =>
    invoke<UpdateDiagnostic>("diagnose_update_for_package", { folderName }),
  packageThumbnail: (folderName) =>
    invoke<string | null>("package_thumbnail", { folderName }),
  fetchChangelog: (pageUrl) =>
    invoke<Changelog>("fetch_changelog", { pageUrl }),

  getSimbriefPilotId: () => invoke<string | null>("get_simbrief_pilot_id"),
  setSimbriefPilotId: (pilotId) =>
    invoke<void>("set_simbrief_pilot_id", { pilotId }),
  refreshSimbrief: () => invoke<SimBriefRefreshResult>("refresh_simbrief"),
  listSimbriefFlights: () => invoke<SimBriefFlight[]>("list_simbrief_flights"),
  deleteSimbriefFlight: (ofpId) =>
    invoke<void>("delete_simbrief_flight", { ofpId }),

  dismissUpdate: (folderName) => invoke<void>("dismiss_update", { folderName }),
  dismissAllUpdates: () => invoke<void>("dismiss_all_updates"),
  clearDismissedUpdates: () => invoke<void>("clear_dismissed_updates"),

  getDashboardStats: () => invoke<DashboardStats>("get_dashboard_stats"),

  listFlightLog: () => invoke<FlightLogEntry[]>("list_flight_log"),
  deleteFlightLogEntry: (id) => invoke<void>("delete_flight_log_entry", { id }),
  debugSeedFlightLog: () => invoke<number>("debug_seed_flight_log"),
  onFlightLogChange: (cb) => listen<unknown>("flightlog://changed", () => cb()),
  getFlightStatus: () => invoke<FlightStatus>("get_flight_status"),
  onFlightStatus: (cb) =>
    listen<FlightStatus>("flight://current", (event) => cb(event.payload)),

  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  setAppSetting: (key, value) => invoke<void>("set_app_setting", { key, value }),
  setAutostart: (enabled) => invoke<boolean>("set_autostart", { enabled }),
  clearCaches: () => invoke<number>("clear_caches"),
  resetSettings: () => invoke<number>("reset_settings"),

  backupCommunity: (destPath) =>
    invoke<BackupResult>("backup_community", { destPath }),
  exportAddons: (destPath, format) =>
    invoke<ExportResult>("export_addons", { destPath, format }),
  async pickSavePath(defaultName, filters) {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({ defaultPath: defaultName, filters });
    return typeof path === "string" ? path : null;
  },
  async pickFolderPath() {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({ multiple: false, directory: true });
    if (result === null) return null;
    return typeof result === "string" ? result : null;
  },
};

// ---------------------------------------------------------------------------
// Demo mode — used only when the app runs in a plain browser (no Tauri bridge).
// Lets us iterate on the UI without compiling the Rust backend.
// ---------------------------------------------------------------------------

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const demoAddons: Addon[] = [
  {
    id: "demo-ltfj",
    source: "sceneryaddons",
    title: "SceneryTR Design – LTFJ Sabiha Gokcen International Airport v1.0.4",
    developer: "SceneryTR Design",
    name: "LTFJ Sabiha Gokcen International Airport",
    version: "1.0.4",
    icao: "LTFJ",
    simulator: "MSFS 2020",
    pageUrl: "https://sceneryaddons.org/",
    downloadMethods: [
      { kind: "torrent", name: "Torrent Download", url: "#" },
      { kind: "mirror",  name: "Mirror #1",        url: "#" },
    ],
    imageUrl: null,
    releasedAt: null,
  },
  {
    id: "demo-rksi",
    source: "sceneryaddons",
    title: "SiamFlight – RKSI Incheon International Airport v0.9.7",
    developer: "SiamFlight",
    name: "RKSI Incheon International Airport",
    version: "0.9.7",
    icao: "RKSI",
    simulator: "MSFS 2020/2024",
    pageUrl: "https://sceneryaddons.org/",
    downloadMethods: [
      { kind: "torrent", name: "Torrent Download", url: "#" },
      { kind: "mirror",  name: "Mirror #1",        url: "#" },
      { kind: "mirror",  name: "Mirror #2",        url: "#" },
    ],
    imageUrl: null,
    releasedAt: null,
  },
  {
    id: "demo-vtsy",
    source: "sceneryaddons",
    title: "Simman – VTSY Betong International Airport v1.1.0",
    developer: "Simman",
    name: "VTSY Betong International Airport",
    version: "1.1.0",
    icao: "VTSY",
    simulator: "MSFS 2020",
    pageUrl: "https://sceneryaddons.org/",
    downloadMethods: [
      { kind: "torrent", name: "Torrent Download", url: "#" },
    ],
    imageUrl: null,
    releasedAt: null,
  },
  {
    id: "demo-kjfk",
    source: "sceneryaddons",
    title: "FlyTampa – KJFK New York John F. Kennedy International Airport v1.2.0",
    developer: "FlyTampa",
    name: "KJFK New York John F. Kennedy International Airport",
    version: "1.2.0",
    icao: "KJFK",
    simulator: "MSFS 2020/2024",
    pageUrl: "https://sceneryaddons.org/",
    downloadMethods: [
      { kind: "torrent", name: "Torrent Download", url: "#" },
      { kind: "mirror",  name: "Mirror #1",        url: "#" },
    ],
    imageUrl: null,
    releasedAt: null,
  },
  {
    id: "demo-simplaza-leib",
    source: "simplaza",
    title: "Iniscene – LEIB Ibiza Airport v2.0.1",
    developer: "Iniscene",
    name: "LEIB Ibiza Airport",
    version: "2.0.1",
    icao: "LEIB",
    simulator: "MSFS 2020",
    pageUrl: "https://simplaza.org/",
    downloadMethods: [
      { kind: "direct", name: "Direct Download", url: "#" },
    ],
    imageUrl: null,
    releasedAt: null,
  },
];

type DemoListener = (job: DownloadJob) => void;
const demoJobs = new Map<string, DownloadJob>();
const demoListeners = new Set<DemoListener>();

function demoEmit(job: DownloadJob) {
  demoJobs.set(job.id, job);
  demoListeners.forEach((cb) => cb(job));
}

const demoApi: Api = {
  async listSources() {
    await sleep(100);
    return [
      { id: "sceneryaddons", name: "SceneryAddons", homeUrl: "https://sceneryaddons.org" },
      { id: "simplaza",      name: "Simplaza",      homeUrl: "https://simplaza.org" },
    ];
  },
  async browseSource(sourceId, page) {
    await sleep(250);
    return {
      addons: demoAddons.filter((a) => a.source === sourceId).slice(0, 6),
      page,
      hasMore: page < 3,
    };
  },
  async search(query, sourceId) {
    await sleep(350);
    const q = query.trim().toLowerCase();
    return demoAddons
      .filter((a) => a.source === sourceId)
      .filter(
        (a) =>
          !q ||
          a.name.toLowerCase().includes(q) ||
          (a.icao?.toLowerCase().includes(q) ?? false) ||
          (a.developer?.toLowerCase().includes(q) ?? false),
      );
  },
  async openExternal(url) {
    window.open(url, "_blank", "noopener,noreferrer");
  },
  async openLocalPath(path) {
    // En modo demo no hay sistema de archivos al que abrir; lo
    // logueamos para que sea obvio durante el desarrollo de UI.
    console.info("[demo] openLocalPath:", path);
  },

  async listDownloads() {
    return [...demoJobs.values()].sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  },
  async startDownload({ addonId, addonTitle, source, method }) {
    const id = crypto.randomUUID();
    const base: DownloadJob = {
      id,
      addonId,
      addonTitle,
      source,
      methodKind: method.kind,
      methodName: method.name,
      url: method.url,
      phase: "queued",
      bytesTotal: 0,
      bytesDone: 0,
      speedBps: 0,
      etaSeconds: null,
      message: null,
      error: null,
      installPath: null,
      createdAt: new Date().toISOString(),
    };
    demoEmit(base);
    await sleep(100);
    if (method.kind === "torrent") {
      demoEmit({
        ...base,
        phase: "error",
        error: "Torrent downloads are coming in the next update.",
      });
    } else {
      demoEmit({
        ...base,
        phase: "completed",
        message: "Opened in your browser (demo).",
      });
    }
    return demoJobs.get(id)!;
  },
  async cancelDownload(id) {
    const j = demoJobs.get(id);
    if (j && j.phase !== "completed") demoEmit({ ...j, phase: "cancelled", message: "Cancelled" });
  },
  async pauseDownload(id) {
    const j = demoJobs.get(id);
    if (j && j.phase === "downloading") {
      demoEmit({ ...j, phase: "paused", message: "Paused", speedBps: 0, etaSeconds: null });
    }
  },
  async resumeDownload(id) {
    const j = demoJobs.get(id);
    if (j && j.phase === "paused") {
      demoEmit({ ...j, phase: "downloading", message: null });
    }
  },
  async clearDownload(id) {
    demoJobs.delete(id);
    // Surface a synthetic "__removed__" update so the store can reconcile.
    demoListeners.forEach((cb) =>
      cb({
        id,
        addonId: "",
        addonTitle: "",
        source: "",
        methodKind: "mirror",
        methodName: "",
        url: "",
        phase: "cancelled",
        bytesTotal: 0,
        bytesDone: 0,
        speedBps: 0,
        etaSeconds: null,
        message: "__removed__",
        error: null,
        installPath: null,
        createdAt: "",
      }),
    );
  },
  async onDownloadUpdate(cb) {
    demoListeners.add(cb);
    return async () => {
      demoListeners.delete(cb);
    };
  },

  async communityFolder() {
    return null;
  },
  async installFromFile() {
    // Sin diálogo nativo — devolvemos un resultado simulado para que
    // la UI pueda probar el camino feliz.
    await sleep(400);
    return {
      packages: [
        {
          name: "demo-package",
          installPath: "C:/Demo/Community/demo-package",
          sizeBytes: 12_345_678,
        },
      ],
      totalBytes: 12_345_678,
      installerPayload: null,
      ptpPayload: null,
    };
  },
  async listInstalled() {
    return [];
  },
  async forgetInstall() {
    // no-op en demo
  },
  async diagnosePmdgOc() {
    return {
      detectedPath: null,
      fromSetting: false,
      triedPaths: ["(modo demo — no se ejecutó la detección real)"],
    };
  },
  async pickFilePath() {
    return null;
  },

  async gsxLookup(icao) {
    // En modo demo simulamos que sólo LFPG y KJFK tienen perfil GSX
    // — alcanza para validar las dos ramas (badge sí / badge no) en
    // el navegador sin tocar la red.
    await sleep(150);
    const upper = icao.trim().toUpperCase();
    if (upper === "LFPG") {
      return [
        {
          id: 1,
          title: "GSX Profile - Asobo LFPG Paris-Charles de Gaulle",
          link: "https://flightsim.to/file/1/",
          thumbnail: null,
          authorName: "DemoAuthor",
          simulator: "MSFS2024",
        },
      ];
    }
    if (upper === "KJFK") {
      return [
        {
          id: 2,
          title: "GSX Profile - FlyTampa KJFK",
          link: "https://flightsim.to/file/2/",
          thumbnail: null,
          authorName: "DemoAuthor",
          simulator: "MSFS2020",
        },
      ];
    }
    return [];
  },

  async checkForUpdate() {
    // En modo demo simulamos siempre que hay una versión nueva — así
    // se puede iterar sobre el banner sin levantar la app real.
    await sleep(120);
    return {
      currentVersion: "0.1.0",
      latestVersion: "0.2.0",
      releaseUrl: "https://github.com/n0xful/SceneryAddonsBrowser/releases/latest",
      assetUrl: null,
      notesMarkdown:
        "## What's new\n\n- **GSX integration**: badge en cada resultado con perfil disponible.\n- **World map**: vista de mundo con clustering verde.\n- Fix de varios crashes al cancelar torrents.\n",
      publishedAt: new Date().toISOString(),
    };
  },
  async installUpdate() {
    // En demo no hay nada que instalar — sólo loggeamos.
    console.info("[demo] installUpdate llamado (no-op)");
  },
  async onUpdateProgress() {
    // En demo no llega ningún evento, sólo devolvemos un unsubscribe
    // vacío para que el caller pueda hacer cleanup sin chequeos extra.
    return async () => {};
  },

  async listAddonsOnMap() {
    await sleep(150);
    // Coordenadas reales para validar la vista de mapa en el navegador
    // sin tocar el backend. Cuatro aeropuertos repartidos por el mundo.
    return [
      {
        addonId: "demo-ltfj",
        source: "sceneryaddons",
        title: "LTFJ Sabiha Gokcen",
        icao: "LTFJ",
        airportName: "Sabiha Gökçen International Airport",
        latitude: 40.898602,
        longitude: 29.30920029,
      },
      {
        addonId: "demo-rksi",
        source: "sceneryaddons",
        title: "RKSI Incheon",
        icao: "RKSI",
        airportName: "Incheon International Airport",
        latitude: 37.46907,
        longitude: 126.45049,
      },
      {
        addonId: "demo-kjfk",
        source: "sceneryaddons",
        title: "KJFK New York JFK",
        icao: "KJFK",
        airportName: "John F Kennedy International Airport",
        latitude: 40.63980103,
        longitude: -73.77890015,
      },
      {
        addonId: "demo-leib",
        source: "simplaza",
        title: "LEIB Ibiza",
        icao: "LEIB",
        airportName: "Ibiza Airport",
        latitude: 38.872898,
        longitude: 1.37312,
      },
    ];
  },
  async refreshAirportsDataset() {
    await sleep(300);
    return 4;
  },

  async scanCommunity() {
    await sleep(250);
    return {
      packages: demoCommunity,
      skippedNoManifest: 0,
      skippedInvalidManifest: 0,
      communityPath: "D:/MSFS/Community (demo)",
    };
  },
  async listCommunityPackages() {
    await sleep(120);
    return demoCommunity;
  },
  async listAvailableUpdates() {
    await sleep(150);
    // En demo simulamos que MDSD tiene update disponible.
    return [
      {
        folderName: "bravoairspace-airport-mdsd-las-americas",
        title: "MDSD Las Americas International Airport",
        icao: "MDSD",
        installedVersion: "1.5.5",
        latestVersion: "1.6.5",
        source: "sceneryaddons",
        addonId: "demo-mdsd",
        pageUrl: "https://sceneryaddons.org/",
      },
    ];
  },
  async refreshUpdatesForInstalled() {
    await sleep(400);
    return {
      icaosChecked: 4,
      queriesRun: 4,
      queriesSkippedCached: 4,
      queriesFailed: 0,
      addonsSeen: 4,
      elapsedMs: 380,
    };
  },
  async uninstallCommunityPackage() {
    await sleep(120);
    // no-op en demo
  },
  async packageThumbnail() {
    return null;
  },
  async getSimbriefPilotId() {
    return null;
  },
  async setSimbriefPilotId() {},
  async refreshSimbrief() {
    return { added: 0, alreadyKnown: true, flight: null };
  },
  async listSimbriefFlights() {
    return [];
  },
  async deleteSimbriefFlight() {},
  async fetchChangelog(pageUrl) {
    return {
      sourceUrl: pageUrl,
      lines: [
        "v2.4.0 — Demo changelog: nuevas texturas y fix de glide path.",
        "v2.3.5 — Demo: corregido cabe en MSFS 2024.",
      ],
    };
  },
  async dismissUpdate() {
    /* no-op demo */
  },
  async dismissAllUpdates() {
    /* no-op demo */
  },
  async clearDismissedUpdates() {
    /* no-op demo */
  },
  async getAppSettings() {
    return {
      showSimbriefLines: true,
      showSimconnectLines: true,
      checkUpdatesOnStart: true,
      minimizeToTray: false,
      onboardingCompleted: true, // demo: skip tour
      defaultView: "dashboard",
      autostartEnabled: false,
      simbriefPilotId: null,
      pmdgOcPath: null,
      communityPath: "C:/Demo/Community",
      logsPath: null,
      appDataPath: null,
    };
  },
  async setAppSetting() {
    /* no-op demo */
  },
  async setAutostart(enabled) {
    return enabled;
  },
  async clearCaches() {
    return 0;
  },
  async resetSettings() {
    return 0;
  },
  async backupCommunity() {
    return {
      outputPath: "C:/Demo/community-backup-demo.zip",
      packageCount: 5,
      totalBytes: 0,
      elapsedMs: 0,
    };
  },
  async exportAddons() {
    return { outputPath: "C:/Demo/export.csv", rowCount: 5 };
  },
  async pickSavePath() {
    return null;
  },
  async pickFolderPath() {
    return null;
  },
  async listFlightLog() {
    return [];
  },
  async deleteFlightLogEntry() {
    /* no-op demo */
  },
  async debugSeedFlightLog() {
    return 0;
  },
  async onFlightLogChange() {
    return async () => {};
  },
  async getFlightStatus() {
    return {
      simRunning: false,
      simconnectConnected: false,
      originIcao: null,
      originName: null,
      destinationIcao: null,
      destinationName: null,
      aircraftIcao: null,
      distanceNm: null,
      currentLat: null,
      currentLon: null,
      currentAltFt: null,
      currentGroundSpeedKt: null,
      onGround: null,
      lastCheckedAt: new Date().toISOString(),
    };
  },
  async onFlightStatus() {
    return async () => {};
  },
  async getDashboardStats() {
    await sleep(120);
    const totalSize = demoCommunity.reduce((acc, p) => acc + (p.sizeBytes ?? 0), 0);
    const byType: { label: string; count: number; sizeBytes: number }[] = [];
    const bumpType = (label: string, bytes: number) => {
      const found = byType.find((b) => b.label === label);
      if (found) {
        found.count++;
        found.sizeBytes += bytes;
      } else {
        byType.push({ label, count: 1, sizeBytes: bytes });
      }
    };
    let airports = 0;
    let liveries = 0;
    let aircraft = 0;
    for (const p of demoCommunity) {
      const ct = (p.contentType ?? "").toUpperCase();
      const bytes = p.sizeBytes ?? 0;
      if (ct === "SCENERY") {
        if (p.icao) airports++;
        bumpType("Aeropuertos", bytes);
      } else if (ct === "AIRCRAFT") {
        if (p.dependenciesCount > 0) {
          liveries++;
          bumpType("Liveries", bytes);
        } else {
          aircraft++;
          bumpType("Aviones", bytes);
        }
      } else if (ct === "MISC") {
        bumpType("Sonido / Misc", bytes);
      } else {
        bumpType("Otros", bytes);
      }
    }
    const creatorCounts = new Map<string, { count: number; bytes: number }>();
    for (const p of demoCommunity) {
      if (!p.creator) continue;
      const acc = creatorCounts.get(p.creator) ?? { count: 0, bytes: 0 };
      acc.count++;
      acc.bytes += p.sizeBytes ?? 0;
      creatorCounts.set(p.creator, acc);
    }
    const topCreators = [...creatorCounts.entries()]
      .map(([creator, v]) => ({ creator, count: v.count, sizeBytes: v.bytes }))
      .sort((a, b) => b.count - a.count || b.sizeBytes - a.sizeBytes);
    const largest = [...demoCommunity]
      .filter((p) => p.sizeBytes != null)
      .sort((a, b) => (b.sizeBytes ?? 0) - (a.sizeBytes ?? 0))
      .slice(0, 10)
      .map((p) => ({
        folderName: p.folderName,
        title: p.title,
        creator: p.creator,
        sizeBytes: p.sizeBytes,
        contentType: p.contentType,
      }));
    return {
      totalPackages: demoCommunity.length,
      totalSizeBytes: totalSize,
      updatesAvailable: 1,
      byType,
      topCreators,
      largestPackages: largest,
      recentlyAdded: largest.slice(0, 5),
      airportsCount: airports,
      liveriesCount: liveries,
      aircraftCount: aircraft,
    };
  },
  async diagnoseUpdateForPackage(folderName) {
    await sleep(80);
    return {
      folderName,
      package: {
        icao: "MDSD",
        packageVersion: "0.1.0",
        contentType: "SCENERY",
        title: "MDSD demo",
      },
      airportMatch: { icao: "MDSD", name: "Las Americas International Airport" },
      catalogEntries: [
        {
          source: "sceneryaddons",
          addonId: "demo-mdsd-1",
          title: "MDSD Las Americas v1.6.5",
          version: "1.6.5",
          lastSeenAt: new Date().toISOString(),
        },
      ],
      cacheEntries: [
        {
          source: "sceneryaddons",
          lastKnownVersion: "1.6.5",
          checkedAt: new Date().toISOString(),
        },
      ],
      blocker: null,
      wouldEmit: {
        installedVersion: "0.1.0",
        latestVersion: "1.6.5",
        source: "sceneryaddons",
      },
    };
  },
};

// Datos que pinta la sidebar del mapa en modo demo. Mezclamos
// paquetes con coords (aeropuertos) y sin coords (sound pack) para
// poder validar ambos render paths en el navegador.
const demoCommunity: CommunityPackage[] = [
  {
    folderName: "bravoairspace-airport-mdsd-las-americas",
    installPath: "D:/MSFS/Community/bravoairspace-airport-mdsd-las-americas",
    title: "MDSD Las Americas International Airport",
    creator: "BravoAirspace",
    contentType: "SCENERY",
    packageVersion: "1.5.5",
    minimumGameVersion: "1.39.9",
    icao: "MDSD",
    sizeBytes: 1_204_567_890,
    folderModifiedAt: "2026-04-22 10:15:00",
    dependenciesCount: 0,
    scannedAt: "2026-04-25 12:00:00",
    airportName: "Las Americas International Airport",
    latitude: 18.4297,
    longitude: -69.6689,
  },
  {
    folderName: "demo-kjfk",
    installPath: "D:/MSFS/Community/demo-kjfk",
    title: "KJFK New York JFK",
    creator: "FlyTampa",
    contentType: "SCENERY",
    packageVersion: "1.2.0",
    minimumGameVersion: "1.30.0",
    icao: "KJFK",
    sizeBytes: 980_000_000,
    folderModifiedAt: "2026-03-10 09:00:00",
    dependenciesCount: 0,
    scannedAt: "2026-04-25 12:00:00",
    airportName: "John F Kennedy International Airport",
    latitude: 40.63980103,
    longitude: -73.77890015,
  },
  {
    folderName: "fbw-a32nx-soundpack",
    installPath: "D:/MSFS/Community/fbw-a32nx-soundpack",
    title: "FlyByWire A32NX Sound Pack",
    creator: "FlyByWire",
    contentType: "MISC",
    packageVersion: "0.9.1",
    minimumGameVersion: "1.30.0",
    icao: null,
    sizeBytes: 250_000_000,
    folderModifiedAt: "2026-02-01 18:30:00",
    dependenciesCount: 0,
    scannedAt: "2026-04-25 12:00:00",
    airportName: null,
    latitude: null,
    longitude: null,
  },
  {
    folderName: "asobo-livery-a320neo-iberia",
    installPath: "D:/MSFS/Community/asobo-livery-a320neo-iberia",
    title: "A320neo Iberia Livery",
    creator: "Asobo",
    contentType: "AIRCRAFT",
    packageVersion: "1.0.0",
    minimumGameVersion: "1.30.0",
    icao: null,
    sizeBytes: 80_000_000,
    folderModifiedAt: "2026-01-12 18:30:00",
    dependenciesCount: 1,
    scannedAt: "2026-04-25 12:00:00",
    airportName: null,
    latitude: null,
    longitude: null,
  },
  {
    folderName: "asobo-aircraft-a320neo",
    installPath: "D:/MSFS/Community/asobo-aircraft-a320neo",
    title: "A320neo Base Aircraft",
    creator: "Asobo",
    contentType: "AIRCRAFT",
    packageVersion: "1.4.0",
    minimumGameVersion: "1.30.0",
    icao: null,
    sizeBytes: 1_200_000_000,
    folderModifiedAt: "2026-01-10 12:00:00",
    dependenciesCount: 0,
    scannedAt: "2026-04-25 12:00:00",
    airportName: null,
    latitude: null,
    longitude: null,
  },
];

export const api: Api = isTauri ? realApi : demoApi;
