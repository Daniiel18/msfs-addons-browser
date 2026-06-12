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
}

export interface InstallerPayload {
  extractedDir: string;
  primaryInstaller: string;
  otherInstallers: string[];
}

/** Vuelo persistido del historial SimBrief. La app va acumulando
 *  uno por refresh contra la API pública (que solo expone el
 *  último OFP). El mapa pinta una LineString por cada uno. */
/** (v3.32.0 #4/#6) Briefing del OFP más reciente de SimBrief: METAR/TAF
 *  reales (vida real) de origen/destino/alterno + NOTAMs. Efímero: se
 *  pide on-demand y sólo se muestra si matchea origen+destino del vuelo. */
export interface SimBriefNotam {
  location: string | null;
  text: string;
}
export interface SimBriefBriefing {
  originIcao: string | null;
  destinationIcao: string | null;
  alternateIcao: string | null;
  origMetar: string | null;
  origTaf: string | null;
  destMetar: string | null;
  destTaf: string | null;
  altnMetar: string | null;
  altnTaf: string | null;
  notams: SimBriefNotam[];
  generatedAt: string | null;
}

/** (v4.11.0) Info liviana de aeropuerto por ICAO (nombre/ciudad/país). */
export interface AirportBrief {
  icao: string;
  name: string;
  municipality: string | null;
  isoCountry: string | null;
}

/** (v4.10.0) Un punto del plan de ruta (navlog de SimBrief). */
export interface RouteFix {
  ident: string;
  fixType: string | null;
  lat: number;
  lon: number;
  stage: string | null;
  isSidStar: boolean;
}

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
  /** (v1.1.0) Pasajeros planificados del OFP. Lo usa el watcher al
   *  OUT para pre-popular `flightLog.passengers` automáticamente. */
  paxCount: number | null;
  /** Carga útil planificada en kg (normalizada desde lbs si aplica). */
  cargoKg: number | null;
  /** Combustible planificado a quemar (taxi+enroute) en kg. */
  fuelBurnKg: number | null;
  /** "lbs" o "kgs" del OFP — sólo trazabilidad. */
  units: string | null;
  /** (v4.17.0) Matrícula (registration) del OFP — usada por la doble
   *  validación del link y mostrada en el modal de confirmación. */
  aircraftReg?: string | null;
  /** (v4.10.0) Puntos del plan de ruta (navlog). Vacío si el OFP no
   *  trae navlog. Se usa para dibujar la ruta planificada en el mapa. */
  routeFixes?: RouteFix[];
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
  /** (v1.1.4) Rumbo verdadero 0..360°. Usado para rotar el icono
   *  del avión en el mapa del FlightBook. */
  currentHeadingDeg: number | null;
  onGround: boolean | null;
  /** (v0.1.25) Fase granular: "preflight" | "engine_running" |
   *  "pushback" | "taxi_out" | "takeoff" | "climbing" | "cruise" |
   *  "descent" | "approach" | "landed_rollout" | "taxi_in" |
   *  "parking" | "deboarding". `null` cuando SimConnect no está
   *  conectado o no se pudo derivar todavía. */
  phaseLabel: string | null;
  /** (v3.0.0) Gate / parking actual cuando el avión está en tierra.
   *  Se popula desde SimConnect Facility Data API al conectar
   *  (request "preflight") y se persiste en el flight_log al OUT.
   *  Ejemplos: "A34", "C7", "Ramp 4", "Gate 12B". */
  currentGate: string | null;
  /** (v3.0.0) ICAO del aeropuerto más cercano al avión (nearest).
   *  Se usa en el frontend para detener el auto-refresh de SimBrief
   *  cuando el plan ya coincide con la posición actual. */
  currentAirportIcao: string | null;
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
  showSimconnectLines: boolean;
  checkUpdatesOnStart: boolean;
  minimizeToTray: boolean;
  onboardingCompleted: boolean;
  defaultView: string;
  /** Tema visual: "dark" o "light". */
  theme: string;
  /** (v3.1.0) Idioma de la UI: "auto" | "es" | "en". Auto resuelve
   *  contra `navigator.language` al primer arranque. */
  language: string;
  /** (v3.39.0 #3) Unidades POR CATEGORÍA. Reactivo en toda la UI
   *  (FlightBook, Weather, Performance, badge). Valores: weight kg|lb,
   *  altitude ft|m, speed kt|kmh|mph, vs fpm|ms, distance nm|km|mi,
   *  pressure inHg|hPa. */
  unitWeight: string;
  unitAltitude: string;
  unitSpeed: string;
  unitVs: string;
  unitDistance: string;
  unitPressure: string;
  /** (v3.28.0 P7.11) Unidad de temperatura: "C" | "F". */
  tempUnit: string;
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
  /** (v3.5.0) `ATC MODEL` — referencia interna del addon. Permite
   *  agrupar vuelos por tipo de avión aunque la livery cambie. */
  aircraftModel: string | null;
  /** (v3.5.0) `ATC AIRLINE` — aerolínea visible configurada en el
   *  Hangar / livery del avión. */
  aircraftAirline: string | null;
  /** (v3.5.0) `ATC ID` — matrícula real ("EC-MXY"). En MSFS 2024
   *  reemplaza a `ATC TAIL NUMBER` (deprecada). */
  aircraftRegistration: string | null;
  distanceNm: number | null;
  /** Duración total entre `startedAt` y `endedAt` en segundos. */
  flightTimeS: number | null;
  maxAltitudeFt: number | null;
  /** Vertical speed (FPM) en el momento del touchdown — negativo
   *  = descenso. -100 a -500 es un "buen aterrizaje"; -600 a
   *  -1000 áspero; bajo -1000 abusivo. `null` si no se capturó. */
  landingFpm: number | null;
  /** Ground speed máxima durante el vuelo (knots). */
  maxGroundSpeedKt: number | null;
  /** True airspeed máxima durante el vuelo (knots). */
  maxTrueAirspeedKt: number | null;
  /** Parking spot / gate de salida — typically "Position: lat,lon"
   *  cuando no detectamos parking específico. */
  departureGate: string | null;
  arrivalGate: string | null;
  /** Pasajeros (manual hasta integración GSX). */
  passengers: number | null;
  /** Carga útil en kg (manual). */
  cargoKg: number | null;
  /** Combustible consumido en kg — automático cuando SimConnect
   *  estaba conectado al OUT (initial fuel) y al IN (final fuel). */
  fuelUsedKg: number | null;
  /** Segundos totales con el sim pausado durante el vuelo. Ya
   *  están restados del `flightTimeS` (block time real). */
  pausedSeconds: number;
  source: string;
  /** (v3.6.0 Phase H) Número de vuelo VA ("DL115"). Para VAS-ACARS
   *  viene del summary; para SimConnect del cross-match con el OFP
   *  de SimBrief al iniciar el vuelo. */
  flightNumber: string | null;
  /** (v3.6.0) Callsign ATC ("DAL115"). Misma fuente que flightNumber. */
  callsign: string | null;
  /** (v3.6.0) ICAO de aerolínea (3 letras: "DAL", "IBE"). Canónico
   *  para agrupar por aerolínea — más robusto que `aircraftAirline`. */
  airlineIcao: string | null;
  /** (v3.6.0) `completed` | `partial`. La UI sólo muestra `completed`. */
  status: string;
  /** (v3.6.0 Epic E) Score total alcanzado (suma de items). */
  scoreTotal: number | null;
  /** (v3.6.0) Score máximo posible (suma de points_max). */
  scoreMax: number | null;
  /** (v3.6.0) Grade derivada del %: 'A'|'B'|'C'|'D'|'F'. */
  scoreGrade: string | null;
  /**
   * (v4.0.0 P7.6b iter 3) `true` si el aterrizaje tuvo rebote
   * (Stream B detectó 2+ flancos 0→1 de SIM_ON_GROUND con
   * radio_alt < 50ft). La UI lo marca con un badge "Bouncing
   * Detected" al lado del FPM. Coincide con la detección de
   * LandingToast.
   */
  bounced: boolean;
  /**
   * (v4.0.0 P7.4b) Temperatura máxima de frenos (°C) durante el vuelo.
   * Solo se puebla si el avión publica el LVar de brake temp (FBW
   * A32NX, etc.) Y el usuario tiene el módulo WASM de MobiFlight
   * instalado (puente LVar opcional). `null` en la gran mayoría de
   * vuelos.
   */
  maxBrakeTempC: number | null;
  /** (v4.16.1 #5b) METAR real de salida/llegada (vida real) capturado del
   *  OFP de SimBrief al finalizar — para consulta histórica. `null` para
   *  imports VAS / vuelos sin OFP. */
  metarOrigin?: string | null;
  metarDest?: string | null;
}

/** (v3.6.0 Phase H — Epic E) Reporte de scoring para un vuelo. */
export interface ScoreReport {
  flightId: number;
  total: number;
  max: number;
  percentage: number;
  grade: string;
  items: ScoreItem[];
}

export interface ScoreItem {
  phase: string;
  ruleId: string;
  label: string;
  pointsEarned: number;
  pointsMax: number;
  passed: boolean;
  severity: string;
  /** (v4.1.0) Estado de checklist: "done" (✓) | "missed" (✗) | "na" (—). */
  status: string;
  evidence: unknown;
}

/** (v3.6.0 Phase H — Epic D) Tag de aerolínea para los chips
 *  horizontales sobre el mapa. */
export interface AirlineTag {
  /** ICAO 3-letter — preferido. `null` para fallback por aircraft_airline. */
  icao: string | null;
  /** Nombre legible mostrado en el chip. */
  name: string;
  /** Cantidad de vuelos completados — para badge "N flights". */
  flightCount: number;
}

/** (v3.6.0 Phase H — Epic D) KPIs agregados de una aerolínea. */
export interface AirlineKpis {
  airlineIcao: string | null;
  airlineName: string | null;
  flightCount: number;
  totalPassengers: number;
  totalCargoKg: number;
  totalFuelKg: number;
  totalDistanceNm: number;
  totalBlockSeconds: number;
  avgLandingFpm: number | null;
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
  /** (v4.24.1) Computado en el backend: true si el paquete es una
   *  librería/complemento (AIRAC, night lights, enhancements,
   *  excludes…) y NO un aeropuerto. Única fuente de verdad compartida
   *  entre el mapa y el conteo del dashboard. */
  isLibraryPack?: boolean;
}

/** Resultado de un scan de la carpeta Community. */
export interface ScanReport {
  packages: CommunityPackage[];
  skippedNoManifest: number;
  skippedInvalidManifest: number;
  communityPath: string;
}

/** (v3.0.0 / v3.1.0) Livery de aircraft addon detectada en Community.
 *  Una entrada por cada `[fltsim.N]` dentro de los `aircraft.cfg` de
 *  cualquier paquete `{vendor}-aircraft-{model}[-liveries]`. Cubre
 *  PMDG, Fenix, iniBuilds, FlyByWire, etc. */
export interface PmdgLivery {
  /** Vendor: "pmdg" | "fnx" | "inibuilds" | "fbw" | "aerosoft" | ... */
  vendor: string;
  /** Modelo a nivel de paquete: "77er" | "77w" | "738" | "320" |
   *  "a350" | "crj" | ... Cuando el paquete contiene múltiples
   *  variantes (CRJ 550/700/900/1000), `variantFolder` distingue. */
  model: string;
  /** Sub-modelo concreto: nombre del subdirectorio en
   *  SimObjects/Airplanes/. Para Aerosoft CRJ aquí aparece
   *  "Aerosoft_CRJ_700", "Aerosoft_CRJ_900", etc. Null cuando hay
   *  un único variant en el paquete. */
  variantFolder: string | null;
  /** Nombre del paquete tal como aparece en Community. */
  packageFolder: string;
  /** `title` de la sección [fltsim.N]. */
  title: string;
  /** `ui_variation` o `ui_type` — nombre amigable. */
  variation: string | null;
  /** `atc_id` — registro / tail number. */
  tailNumber: string | null;
  /** `atc_airline` — operador en plain text. */
  airline: string | null;
  /** `texture` — subcarpeta del livery. */
  texture: string | null;
  /** Data URL del thumbnail (base64 JPG/PNG) o `null` si el livery
   *  no trae imagen renderizable. El backend filtra DDS porque el
   *  WebView no las soporta. */
  thumbnailDataUrl: string | null;
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

/** Input para `update_flight_log_entry` (v0.1.25). Cada campo es
 *  opcional — el frontend envía sólo los que el usuario editó.
 *  `null` en un Option significa "borrar este campo" para strings;
 *  para números no aplica, simplemente omites la clave. */
export interface UpdateFlightInput {
  flightTimeS?: number | null;
  passengers?: number | null;
  cargoKg?: number | null;
  fuelUsedKg?: number | null;
  departureGate?: string | null;
  arrivalGate?: string | null;
  /** (v3.5.0 F2) Matrícula del avión. Necesaria para que el lookup
   *  de foto en planespotters funcione cuando el livery no contenía
   *  un tail real (ej. liveries Fenix con códigos internos). */
  aircraftRegistration?: string | null;
  /** (v4.0.0 — P6.2) ICAO de origen/destino editables. El watcher
   *  a veces detecta mal el aeropuerto de spawn, o un vuelo VAS
   *  importado trae un ICAO erróneo. El backend hace .to_uppercase()
   *  antes de persistir. */
  originIcao?: string | null;
  destinationIcao?: string | null;
}

/** (v3.5.0 F2) Reporte tras borrar todos los vuelos de una fuente. */
export interface DeleteBySourceReport {
  flightsDeleted: number;
  tracksDeleted: number;
}

/** Punto individual de la traza de un vuelo (v0.1.23). Una fila
 *  por muestreo (~cada 10s) en `flight_log_track`. El frontend los
 *  pide para pintar la ruta real del vuelo en el mapa. */
export interface FlightTrackPoint {
  lat: number;
  lon: number;
  altFt: number | null;
  gsKt: number | null;
  ts: string;
}

/** (v4.0.0 P7.9b) Weather capturado por sample durante el vuelo.
 *  Lo lee el Weather modal para dibujar viento/temp/precip reales
 *  de vuelos pasados sin depender de APIs de clima histórico. */
export interface WeatherSample {
  ts: string;
  lat: number;
  lon: number;
  altFt: number | null;
  windDirDeg: number | null;
  windSpeedKt: number | null;
  oatC: number | null;
  baroHpa: number | null;
  visibilityM: number | null;
  precipState: number | null;
  /** (v3.19.0 P7.9c) Cobertura de nubes REAL (Open-Meteo) %, 0..100.
   *  Capturada durante el vuelo en la posición del avión. */
  cloudCoverPct: number | null;
  cloudLowPct: number | null;
  cloudMidPct: number | null;
  cloudHighPct: number | null;
}

/** (v3.26.0 — P7.10) Veredicto de daños/forzado del vuelo. */
export interface DamageReport {
  verdict: "clean" | "forced" | "damaged" | "no_data";
  hasData: boolean;
  overspeedSamples: number;
  stallSamples: number;
  oilPressSamples: number;
  maxG: number | null;
  minG: number | null;
  reasons: string[];
}

/** (v4.0.0 — P5) Punto completo con telemetría extendida para el
 *  Performance modal. Incluye attitude, lights/gear/flaps y 6
 *  métricas × 4 engine slots. Para vuelos VAS imports muchos
 *  campos vienen `null` — el modal detecta series vacías y oculta
 *  las curvas correspondientes. */
export interface FlightTrackFullPoint {
  ts: string;
  lat: number;
  lon: number;
  altFt: number | null;
  gsKt: number | null;
  iasKt: number | null;
  vsFpm: number | null;
  pitchDeg: number | null;
  bankDeg: number | null;
  gForce: number | null;
  flapsPct: number | null;
  gearDown: number | null;
  spoilersPct: number | null;
  engN11: number | null;
  engN12: number | null;
  engN13: number | null;
  engN14: number | null;
  engN21: number | null;
  engN22: number | null;
  engN23: number | null;
  engN24: number | null;
  engEgt1: number | null;
  engEgt2: number | null;
  engEgt3: number | null;
  engEgt4: number | null;
  engFfPph1: number | null;
  engFfPph2: number | null;
  engFfPph3: number | null;
  engFfPph4: number | null;
  engOilTemp1: number | null;
  engOilTemp2: number | null;
  engOilTemp3: number | null;
  engOilTemp4: number | null;
  engOilPress1: number | null;
  engOilPress2: number | null;
  engOilPress3: number | null;
  engOilPress4: number | null;
}

/** Reporte de la desinstalación total (v0.1.19). El backend borra
 *  el folder en CADA Community detectada + carpetas extras
 *  (Documents\MSFS Sceneries\…). Devuelve qué paths se eliminaron
 *  y cuáles fallaron para que la UI pueda enseñar el resultado. */
export interface UninstallReport {
  folderName: string;
  removedPaths: string[];
  failedPaths: { path: string; error: string }[];
  dbRowsCleared: number;
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

/** (v2.0.0) Resultado de instalar perfil(es) GSX. Espejo de
 *  `commands::gsx::GsxInstallReport`. `archiveKind` es "single"
 *  (un .ini/.py suelto), "zip" o "rar". */
export interface GsxInstallReport {
  archiveKind: string;
  installedFiles: string[];
  skippedFiles: string[];
}

/** (v2.1.0) Item detectado dentro de un archivo dropeado. Espejo de
 *  `drop_install::DropItem`. */
export interface DropItem {
  /** "gsx_profile" | "community_package" | "installer_exe" | "unknown" */
  kind: string;
  label: string;
  icao: string | null;
  /** (v2.1.1) Variantes detectadas (VDGS, noVDGS, handler, locale, etc). */
  variants: string[];
  sourcePath: string;
  relativePath: string;
  sizeBytes: number;
  /** (v2.1.1) Primeras líneas del .ini, útil para distinguir items
   *  con el mismo filename pero diferente contenido. */
  description: string | null;
}

/** (v2.1.0) Resultado de la inspección de un archivo dropeado. */
export interface DropInspection {
  sessionId: string;
  archivePath: string;
  items: DropItem[];
  /** True si el archivo era un .ini/.py suelto. Skip modal. */
  isSingle: boolean;
}

/** (v2.1.0) Resultado de instalar los items seleccionados. */
export interface DropCommitReport {
  installedGsx: string[];
  installedPackages: string[];
  errors: string[];
}

/** (v2.0.0) Estado de la integración con Google Drive. Espejo de
 *  `cloud_sync::CloudConfig`. */
export interface CloudConfig {
  connected: boolean;
  hasCredentials: boolean;
  userEmail: string | null;
  lastSyncAt: string | null;
}

/** (v2.0.0) Inicio del flow OAuth — la URL que la UI abre en el
 *  navegador del usuario y el puerto del listener loopback. */
export interface CloudOauthStart {
  authUrl: string;
  redirectUri: string;
  port: number;
}

/** (v2.0.0) Evento emitido al terminar el flow OAuth. */
/** (v4.17.0) Resultado del link de OFP con doble validación de matrícula.
 *  `status`: "linked" (guardado OK) | "regMismatch" (conflicto — la UI
 *  debe pedir la última confirmación y reintentar con force=true). */
export interface LinkOutcome {
  status: "linked" | "regMismatch";
  simbriefReg: string | null;
  simReg: string | null;
}

export interface CloudOauthCompletedEvent {
  ok: boolean;
  userEmail: string | null;
  error: string | null;
}

/** (v4.14.0 #2) Progreso de transferencia cloud (evento `cloud://progress`).
 *  La velocidad (MB/s) y el ETA los deriva el frontend de los deltas de
 *  `transferred` entre eventos consecutivos. */
export interface CloudProgress {
  /** "upload" | "download". */
  phase: "upload" | "download";
  /** Bytes transferidos hasta ahora. */
  transferred: number;
  /** Tamaño total en bytes. 0 = desconocido. */
  total: number;
  /** true en el último evento de la fase. */
  done: boolean;
}

/** (v2.0.0) Reporte tras un sync exitoso — cuenta uploads y
 *  downloads por colección. */
export interface CloudSyncReport {
  uploadedFlights: number;
  uploadedTracks: number;
  uploadedSettings: number;
  downloadedFlights: number;
  downloadedTracks: number;
  downloadedSettings: number;
}

/** (v3.5.0 F3) Reporte del upload-only — solo conteos enviados. */
export interface CloudUploadReport {
  uploadedFlights: number;
  uploadedTracks: number;
  uploadedSettings: number;
}

/** (v3.5.0 F3) Reporte del download-missing — los nuevos vuelos que
 *  se trajeron del cloud y los que se omitieron por estar ya en local. */
export interface CloudDownloadReport {
  cloudFlights: number;
  newFlights: number;
  skippedExisting: number;
  newTracks: number;
  newSettings: number;
}

/** (v2.0.2) Un paso del diagnóstico de Cloud Sync. Espejo de
 *  `cloud_sync::CloudTestStep`. */
export interface CloudTestStep {
  name: string;
  ok: boolean;
  detail: string;
}

/** (v2.0.2) Reporte completo del diagnóstico. */
export interface CloudTestReport {
  overallOk: boolean;
  steps: CloudTestStep[];
  hint: string | null;
}

/** (v3.5.0) Reporte de la purga del cloud. Conteos del snapshot
 *  remoto antes de borrarlo + flag si el archivo existía. */
export interface CloudPurgeReport {
  deletedFlights: number;
  deletedTracks: number;
  deletedSettings: number;
  /** True si encontramos y borramos el snapshot file de Drive.
   *  False si no había snapshot (purga = no-op exitoso). */
  fileDeleted: boolean;
}

/** (v3.5.0) Reporte de la importación del LOGBOOK.BIN de MSFS.
 *  `sourcePath` y `sim` son `null` cuando no se encontró logbook. */
export interface MsfsLogbookImportReport {
  sourcePath: string | null;
  /** "msfs-2020" | "msfs-2024" | null si no se encontró logbook. */
  sim: string | null;
  candidatesFound: number;
  importedCount: number;
  skippedDuplicates: number;
}

/** (v3.5.0 F2) Candidato VAS-ACARS detectado en disco — un archivo
 *  `.bin` por vuelo. */
export interface VasFlightCandidate {
  uuid: string;
  path: string;
  sizeBytes: number;
}

/** (v3.5.0 F2) Reporte del import de VAS-ACARS. `sourceDir` null
 *  significa que VAS-ACARS no está instalado (carpeta no existe). */
export interface VasImportReport {
  sourceDir: string | null;
  candidatesFound: number;
  importedCount: number;
  /** Vuelos que ya existían (por external_id) y se actualizaron con
   *  campos derivados nuevos (matrícula, airline, gate). Permite
   *  re-import sin perder ediciones manuales de timestamps. */
  updatedCount: number;
  skippedDuplicates: number;
  /** Archivos que no pudieron parsearse (corruptos, versión
   *  incompatible, etc.). Para diagnóstico — no son errores fatales. */
  skippedInvalid: number;
}

/** (v2.0.1) Estado del folder sync — alternativa simple a OAuth.
 *  Espejo de `cloud_sync::FolderSyncConfig`. */
export interface FolderSyncConfig {
  folderPath: string | null;
  lastSyncAt: string | null;
  folderExists: boolean;
  dataFileExists: boolean;
}

/** (v2.0.1) Reporte tras un save al folder. */
export interface FolderSyncSaveReport {
  outputPath: string;
  bytesWritten: number;
  flights: number;
  tracks: number;
  settings: number;
}

/** (v2.0.1) Reporte tras un load desde folder. */
export interface FolderSyncLoadReport {
  sourcePath: string;
  bytesRead: number;
  restoredFlights: number;
  restoredTracks: number;
  restoredSettings: number;
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
