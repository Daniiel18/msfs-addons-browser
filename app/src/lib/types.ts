export type DownloadKind = "torrent" | "mirror" | "direct";

export interface DownloadMethod {
  kind: DownloadKind;
  name: string;
  url: string;
}

export interface Addon {
  id: string;
  source: string;
  title: string;
  developer: string | null;
  name: string;
  version: string | null;
  icao: string | null;
  simulator: string;
  pageUrl: string;
  downloadMethods: DownloadMethod[];
  /** Featured thumbnail scraped from the source's search page, if present. */
  imageUrl: string | null;
  /** Fecha de publicación scrapeada del listado. ISO-8601 cuando
   *  el `<time datetime>` estaba presente; texto crudo si sólo
   *  había contenido visible; `null` cuando el parser no halló nada. */
  releasedAt: string | null;
}

export interface SourceDescriptor {
  id: string;
  name: string;
  homeUrl: string;
}

/** Página de catálogo cuando el usuario no ha tecleado búsqueda. */
export interface BrowsePage {
  addons: Addon[];
  page: number;
  hasMore: boolean;
}

export type DownloadPhase =
  | "queued"
  | "resolving"
  | "downloading"
  | "paused"
  | "installing"
  | "completed"
  | "cancelled"
  | "error";

export interface DownloadJob {
  id: string;
  addonId: string;
  addonTitle: string;
  source: string;
  methodKind: DownloadKind;
  methodName: string;
  url: string;
  phase: DownloadPhase;
  bytesTotal: number;
  bytesDone: number;
  speedBps: number;
  etaSeconds: number | null;
  message: string | null;
  error: string | null;
  installPath: string | null;
  createdAt: string;
}

export interface CommunityInfo {
  path: string;
  variant: "msstore" | "steam";
  exists: boolean;
}

/** Resultado de extraer + copiar un archivo a Community. Espejo del
 *  `InstallResult` de Rust (`install/mod.rs`). */
export interface InstalledPackage {
  name: string;
  installPath: string;
  sizeBytes: number;
}

export interface InstallResult {
  packages: InstalledPackage[];
  totalBytes: number;
  /** Si el archivo era un instalador (.exe/.msi) en vez de un
   *  paquete MSFS. Cuando viene poblado, `packages` está vacío y
   *  el frontend debe ofrecer al usuario abrir el instalador. */
  installerPayload: InstallerPayload | null;
  /** Si el archivo contenía liveries PMDG (.ptp). La app intenta
   *  instalarlas directamente en el paquete PMDG dentro de Community
   *  (copiando texturas + mergeando aircraft.cfg + actualizando
   *  layout.json) — replicando lo que hace PMDG Operations Center
   *  pero sin requerir abrirlo. Las que no se pudieron instalar
   *  quedan en `inboxDir` como fallback manual. */
  ptpPayload: PtpPayload | null;
}

export interface InstallerPayload {
  extractedDir: string;
  primaryInstaller: string;
  otherInstallers: string[];
}

export interface PtpPayload {
  inboxDir: string;
  ptpFiles: string[];
  /** Para cada `ptpFiles[i]`, el aircraft detectado heurísticamente
   *  (nombre propio + nombre del archivo padre + contenido del ZIP),
   *  ej. "PMDG 737-800". `null` cuando no se pudo determinar. */
  detectedAircraft: (string | null)[];
  /** Path al directorio del avión PMDG donde quedó instalada la
   *  livery (`<community>/pmdg-aircraft-XXX/SimObjects/Airplanes/PMDG
   *  XXX/`). Cuando está populado, la livery aparece automáticamente
   *  al elegir la aeronave en MSFS — el usuario no tiene que abrir
   *  PMDG Operations Center. `null` si no se detectó aircraft o si
   *  el paquete PMDG no está instalado en Community. */
  autoInstalledAt: (string | null)[];
}

/** Vuelo persistido del historial SimBrief. La app va acumulando
 *  uno por refresh contra la API pública (que solo expone el
 *  último OFP). El mapa pinta una LineString por cada uno. */
export interface SimBriefFlight {
  ofpId: string;
  pilotId: string;
  flightNumber: string | null;
  callsign: string | null;
  aircraftIcao: string | null;
  originIcao: string;
  originName: string | null;
  originLat: number;
  originLon: number;
  destinationIcao: string;
  destinationName: string | null;
  destinationLat: number;
  destinationLon: number;
  route: string | null;
  distanceNm: number | null;
  estTimeEnrouteS: number | null;
  generatedAt: string | null;
  fetchedAt: string;
}

export interface SimBriefRefreshResult {
  added: number;
  alreadyKnown: boolean;
  flight: SimBriefFlight | null;
}

/** Estado del watcher de "vuelo en curso". Dos capas:
 *  · `simRunning` — proceso MSFS detectado (sysinfo).
 *  · `simconnectConnected` — handshake real con SimConnect.dll;
 *    los campos `currentLat/Lon/AltFt/GroundSpeedKt/onGround`
 *    están poblados en vivo (refresco cada segundo).
 *  El cruce con la última OFP de SimBrief sigue rellenando los
 *  campos de origen/destino aunque SimConnect no esté disponible. */
export interface FlightStatus {
  simRunning: boolean;
  simconnectConnected: boolean;
  originIcao: string | null;
  originName: string | null;
  destinationIcao: string | null;
  destinationName: string | null;
  aircraftIcao: string | null;
  distanceNm: number | null;
  /** Posición en vivo del user aircraft (sólo con SimConnect). */
  currentLat: number | null;
  currentLon: number | null;
  currentAltFt: number | null;
  currentGroundSpeedKt: number | null;
  onGround: boolean | null;
  lastCheckedAt: string;
}

/** Resultado de un backup de la carpeta Community. */
export interface BackupResult {
  outputPath: string;
  packageCount: number;
  totalBytes: number;
  elapsedMs: number;
}

/** Resultado de un export del inventario. */
export interface ExportResult {
  outputPath: string;
  rowCount: number;
}

export type ExportFormat = "csv" | "txt" | "json";

/** Snapshot de las preferencias de la app. Lo devuelve
 *  `getAppSettings`; el frontend lo guarda en `useSettingsStore`. */
export interface AppSettings {
  showSimbriefLines: boolean;
  showSimconnectLines: boolean;
  checkUpdatesOnStart: boolean;
  minimizeToTray: boolean;
  onboardingCompleted: boolean;
  defaultView: string;
  autostartEnabled: boolean;
  simbriefPilotId: string | null;
  communityPath: string | null;
  logsPath: string | null;
  appDataPath: string | null;
}

/** Vuelo registrado por el watcher de SimConnect. A diferencia de
 *  `SimBriefFlight` (que captura *planes*), esto captura el vuelo
 *  efectivamente volado. `endedAt = null` indica un vuelo en curso
 *  o interrumpido (la app cerró antes del aterrizaje). */
export interface FlightLogEntry {
  id: number;
  startedAt: string;
  endedAt: string | null;
  originLat: number;
  originLon: number;
  originIcao: string | null;
  originName: string | null;
  destinationLat: number | null;
  destinationLon: number | null;
  destinationIcao: string | null;
  destinationName: string | null;
  aircraftTitle: string | null;
  aircraftAtcType: string | null;
  distanceNm: number | null;
  flightTimeS: number | null;
  maxAltitudeFt: number | null;
  source: string;
}

/** Estadísticas agregadas que pinta la vista «Dashboard». Sale de
 *  `community_packages` + `compute_available`; el backend hace todo
 *  el group-by en SQL para que la UI sólo renderice. */
export interface DashboardStats {
  totalPackages: number;
  totalSizeBytes: number;
  updatesAvailable: number;
  byType: TypeStat[];
  topCreators: CreatorStat[];
  largestPackages: PackageRef[];
  recentlyAdded: PackageRef[];
  airportsCount: number;
  liveriesCount: number;
  aircraftCount: number;
}

export interface TypeStat {
  label: string;
  count: number;
  sizeBytes: number;
}

export interface CreatorStat {
  creator: string;
  count: number;
  sizeBytes: number;
}

export interface PackageRef {
  folderName: string;
  title: string;
  creator: string | null;
  sizeBytes: number | null;
  contentType: string | null;
}

/** Punto serializado por `airports::list_addons_on_map` — un addon
 *  del catálogo local cuyo ICAO existe en la tabla de aeropuertos.
 *  Lo consume la vista de mapa. */
export interface AddonOnMap {
  addonId: string;
  source: string;
  title: string;
  icao: string;
  airportName: string;
  latitude: number;
  longitude: number;
}

/** Paquete real que vive en el folder Community, leído por el
 *  scanner. `latitude`/`longitude` se rellenan cuando el ICAO es
 *  resolvible contra la tabla de aeropuertos. */
export interface CommunityPackage {
  folderName: string;
  installPath: string;
  title: string;
  creator: string | null;
  contentType: string | null;
  packageVersion: string | null;
  minimumGameVersion: string | null;
  icao: string | null;
  sizeBytes: number | null;
  folderModifiedAt: string | null;
  /** Cuántas dependencies declara el manifest. AIRCRAFT con
   *  deps>0 son liveries (dependen del aircraft base). */
  dependenciesCount: number;
  scannedAt: string;
  airportName: string | null;
  latitude: number | null;
  longitude: number | null;
}

/** Resultado de un scan de la carpeta Community. */
export interface ScanReport {
  packages: CommunityPackage[];
  skippedNoManifest: number;
  skippedInvalidManifest: number;
  communityPath: string;
}

/** Una actualización detectada al cruzar Community con el catálogo. */
export interface AvailableUpdate {
  folderName: string;
  title: string;
  icao: string;
  installedVersion: string;
  latestVersion: string;
  source: string;
  addonId: string;
  pageUrl: string;
}

/** Changelog scrapeado del detail page. Lista de líneas en orden. */
export interface Changelog {
  sourceUrl: string;
  lines: string[];
}

/** Diagnóstico exhaustivo del estado de detección de updates para
 *  un paquete concreto. Lo devuelve `diagnose_update_for_package`.
 *  La UI lo consume para enseñar al usuario qué eslabón rompe la
 *  cadena cuando una update no aparece. */
export interface UpdateDiagnostic {
  folderName: string;
  package: DiagPackage | null;
  airportMatch: DiagAirport | null;
  catalogEntries: DiagCatalog[];
  cacheEntries: DiagCache[];
  blocker: string | null;
  wouldEmit: DiagWouldEmit | null;
}

export interface DiagPackage {
  icao: string | null;
  packageVersion: string | null;
  contentType: string | null;
  title: string;
}

export interface DiagAirport {
  icao: string;
  name: string;
}

export interface DiagCatalog {
  source: string;
  addonId: string;
  title: string;
  version: string | null;
  lastSeenAt: string;
}

export interface DiagCache {
  source: string;
  lastKnownVersion: string | null;
  checkedAt: string;
}

export interface DiagWouldEmit {
  installedVersion: string;
  latestVersion: string;
  source: string;
}

/** Resumen del refresh activo de updates. */
export interface RefreshSummary {
  icaosChecked: number;
  queriesRun: number;
  queriesSkippedCached: number;
  queriesFailed: number;
  addonsSeen: number;
  elapsedMs: number;
}

/** Información de actualización publicada en GitHub Releases.
 *  Espejo de `updater::UpdateInfo` (Rust). Sólo llega al frontend
 *  cuando hay una versión estrictamente mayor. */
export interface UpdateInfo {
  currentVersion: string;
  latestVersion: string;
  releaseUrl: string;
  /** URL del primer asset Windows (.msix/.msi/.exe). `null` cuando la
   *  release todavía no tiene binarios publicados. */
  assetUrl: string | null;
  /** Cuerpo Markdown de la release tal como llega de la API. */
  notesMarkdown: string;
  /** Fecha ISO-8601 de publicación. */
  publishedAt: string | null;
}

/** Perfil GSX Pro disponible en flightsim.to para un ICAO concreto.
 *  Espejo serializado de `gsx::GsxProfile` (Rust) — el comando
 *  `gsx_lookup` devuelve cero o varios. */
export interface GsxProfile {
  id: number;
  title: string;
  link: string;
  thumbnail: string | null;
  authorName: string | null;
  simulator: string | null;
}

/** Fila del historial persistente. Espejo de `repo::InstalledAddonRow`. */
export interface InstalledAddon {
  id: string;
  /** ID del addon de origen, si esta instalación nació de un torrent
   *  resuelto desde el catálogo. `null` para instalaciones manuales. */
  addonId: string | null;
  /** ID del source ("sceneryaddons", "simplaza"), si aplica. */
  source: string | null;
  /** Título legible — para manuales, suele ser el nombre del archivo. */
  title: string;
  /** Carpeta dentro de Community (la que copiamos). */
  name: string | null;
  developer: string | null;
  version: string | null;
  installPath: string;
  sizeBytes: number | null;
  /** ISO-8601 (UTC) — viene como TEXT desde SQLite. */
  installedAt: string;
}
