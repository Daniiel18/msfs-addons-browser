/**
 * (v3.4.0) Sistema i18n centralizado.
 *
 * Arquitectura:
 *   · Diccionarios estáticos ES/EN. Claves dot.notation
 *     ("settings.theme.title").
 *   · `t(key)` hace lookup en el dict del idioma activo; cae a EN; si
 *     tampoco existe, devuelve la clave cruda.
 *   · El idioma activo vive en `useSettingsStore.settings.language`
 *     (persistido en SQLite). **Default `"auto"`** → resuelve contra
 *     `navigator.language` (locale del SO en Tauri 2).
 *   · **(v3.4.0) `preloadLocale()` lee `localStorage["simfleet.locale"]`
 *     ANTES del primer render** — sin esto el shell se montaba en EN
 *     porque el bootstrap de DB es async y `setActiveLocale` corría
 *     después del primer paint. La store mirror al guardar / al
 *     bootstrappear vía `persistLocale()`.
 *   · Cambio manual de idioma persiste a DB + localStorage y dispara
 *     un modal "reinicia para aplicar" — no re-renderizamos en
 *     caliente porque los `t()` son llamados al render-time y
 *     evitamos forzar un reload reactivo de toda la app.
 */

export type Locale = "es" | "en";
export type LocaleSetting = Locale | "auto";

/** Key del localStorage usado por preloadLocale / persistLocale. */
const LS_KEY = "simfleet.locale";

const DICTIONARIES: Record<Locale, Record<string, string>> = {
  es: {
    "common.cancel": "Cancelar",
    "common.save": "Guardar",
    "common.close": "Cerrar",
    "common.delete": "Eliminar",
    "common.confirm": "Confirmar",
    "common.loading": "Cargando…",
    "common.error": "Error",
    "common.errors": "errores",
    "common.search": "Buscar",
    "common.restart": "Reiniciar",
    "common.previous": "← Anterior",
    "common.next": "Siguiente →",
    "common.page": "Página",
    "common.ready": "Listo",
    "common.open": "Abrir",
    "common.refresh": "Refrescar",
    "common.install": "Instalar",
    "common.skip": "Saltar",
    "common.got_it": "Entendido",
    "common.unknown": "desconocido",

    "nav.dashboard": "Dashboard",
    "nav.search": "Buscar",
    "nav.map": "Mapa (escenarios)",
    "nav.addons": "Addons",
    "nav.flightbook": "FlightBook",

    "header.tagline": "FlightBook · SimBrief · SimConnect · GSX",
    "header.settings.tooltip": "Configuración",

    "search.placeholder.scenery":
      "Busca por aeropuerto, ICAO o desarrollador…",
    "search.placeholder.simplaza":
      "Busca por avión, livery, mod o autor…",

    "demo.title":
      "Modo demo (navegador). Instala Rust y ejecuta {cmd} para habilitar la búsqueda real.",

    "splash.task.update_check": "Buscar actualización de la app",
    "splash.task.sources": "Cargar fuentes",
    "splash.task.settings": "Cargar configuración",
    "splash.task.simbrief": "Cargar SimBrief",
    "splash.task.flightlog": "Cargar Flight Log",
    "splash.task.downloads": "Suscribir descargas",
    "splash.task.scan_community": "Escanear carpeta Community",
    "splash.task.refresh_updates": "Buscar actualizaciones de addons",
    "splash.task.preload_catalogs": "Pre-cargar catálogos",

    "splash.tagline.0": "Preparando catálogo de addons…",
    "splash.tagline.1": "Sincronizando carpeta Community…",
    "splash.tagline.2": "Cargando aeropuertos del mundo…",
    "splash.tagline.3": "Calentando motores…",
    "splash.tagline.4": "Verificando últimas versiones…",
    "splash.tagline.5": "Trazando rutas en el mapa…",

    "splash.finishing": "Finalizando",

    "update.banner.new_version": "Nueva versión disponible: v{version}",
    "update.banner.body":
      "Estás en v{current}. Recomendamos actualizar antes de seguir.",
    "update.banner.view_changes": "Ver cambios de esta versión",
    "update.banner.see_full_release": "(ver release completo)",
    "update.banner.auto_restart": "Reiniciar la app automáticamente al terminar",
    "update.banner.install_now": "Instalar ahora",
    "update.progress.downloading": "Descargando v{version}",
    "update.progress.downloading_pct": "Descargando v{version} · {pct}%",
    "update.progress.complete":
      "Descarga completa — lanzando el instalador. La app se cerrará en unos segundos.",

    "settings.title": "Configuración",
    "settings.section.general": "General",
    "settings.section.flights": "Vuelos",
    "settings.section.map_display": "Mostrar en mapa (FlightBook)",
    "settings.section.folders": "Carpetas",
    "settings.section.backup": "Backup",
    "settings.section.gsx": "GSX Pro",
    "settings.section.cloud": "Sincronización con Google Drive",
    "settings.section.import": "Importar inventario",
    "settings.section.export": "Exportar inventario",
    "settings.section.storage": "Almacenamiento",
    "settings.section.tour": "Tour de bienvenida",
    "settings.section.about": "Acerca de",
    "settings.section.appearance": "Apariencia",
    "settings.section.behavior": "Comportamiento",
    "settings.section.language": "Idioma",
    "settings.section.simbrief": "SimBrief",

    "settings.language.title": "Idioma · Language",
    "settings.language.description":
      "«Auto» usa el idioma del sistema operativo. Al cambiar manualmente la app pedirá reiniciar para aplicar los textos.",
    "settings.language.auto": "Auto (sistema)",
    "settings.language.es": "Español",
    "settings.language.en": "English",

    "settings.theme.title": "Tema visual",
    "settings.theme.description":
      "Oscuro o claro. El claro reemplaza fondos, cards y textos conservando los acentos de color. El globo de FlightBook siempre usa tiles satelitales.",
    "settings.theme.dark": "Oscuro",
    "settings.theme.light": "Claro",

    "settings.autostart.title": "Arrancar con Windows",
    "settings.autostart.hint":
      "La app se abre automáticamente al iniciar sesión.",
    "settings.updates.title": "Comprobar updates al arrancar",
    "settings.updates.hint":
      "Verifica nuevas versiones de la app en GitHub al iniciar.",
    "settings.tray.title": "Minimizar a la bandeja al cerrar",
    "settings.tray.hint":
      "Al pulsar la X, la app se oculta en la bandeja del sistema. Click derecho en el icono → Salir.",

    "restart.title": "Reinicia SimFleet",
    "restart.body":
      "El idioma se actualizó. Para que todos los textos cambien (incluyendo los módulos cargados antes del cambio), cierra y vuelve a abrir SimFleet.",
    "restart.button": "Entendido",

    "flying.preflight": "Pre-vuelo",
    "flying.taxi_out": "Rodaje salida",
    "flying.takeoff": "Despegue",
    "flying.climbing": "Ascenso",
    "flying.cruise": "Crucero",
    "flying.descent": "Descenso",
    "flying.approach": "Aproximación",
    "flying.landed_rollout": "Aterrizaje",
    "flying.taxi_in": "Rodaje llegada",
    "flying.parking": "En gate",
    "flying.deboarding": "Desembarque",
    "flying.pushback": "Pushback",
    "flying.engine_running": "Motores encendidos",

    // FlightBook
    "fb.title": "FlightBook",
    "fb.subtitle.list": "Bitácora de vuelos reales.",
    "fb.subtitle.detail": "Detalle del vuelo — ruta real grabada cada 10s.",
    "fb.back_to_globe": "Volver al globo",
    "fb.back_to_globe.tooltip":
      "Volver a la vista de globo con todos los vuelos",
    "fb.empty.title": "Aún no hay vuelos registrados",
    "fb.empty.body":
      "Cuando despegues en MSFS con SimConnect activo, el watcher creará una entrada automáticamente.",
    "fb.flying_now": "Volando ahora",
    "fb.selected_flight": "Vuelo seleccionado",
    "fb.real_route": "Ruta real (cada 10s)",
    "fb.history": "Historial",
    "fb.peak.altitude": "Altitud máx",
    "fb.peak.speed": "Velocidad máx",
    "fb.stat.flights": "Vuelos",
    "fb.stat.total_time": "Tiempo total",
    "fb.stat.distance": "Distancia",
    "fb.stat.passengers": "Pasajeros",
    "fb.stat.cargo": "Carga",
    "fb.stat.total_fuel": "Combustible total",
    "fb.stat.flights.suffix": "Block-to-block grabado",
    "fb.stat.time.suffix": "Block-to-block sumado",
    "fb.block.route": "Ruta",
    "fb.block.times": "Tiempos",
    "fb.block.load": "Carga (SimBrief)",
    "fb.block.aircraft": "Aeronave",
    "fb.block.peak": "Métricas pico",
    "fb.route.origin": "Origen",
    "fb.route.destination": "Destino",
    "fb.route.alternate": "Alterno",
    "fb.route.distance": "Distancia",
    "fb.route.dep_gate": "Gate salida",
    "fb.route.arr_gate": "Gate llegada",
    "fb.times.out": "OUT (block-out)",
    "fb.times.off": "OFF (despegue)",
    "fb.times.on": "ON (toque)",
    "fb.times.in": "IN (block-in)",
    "fb.times.block": "Block",
    "fb.times.flight": "Vuelo (aire)",
    "fb.times.paused": "Pausa",
    "fb.load.passengers": "Pasajeros",
    "fb.load.cargo": "Carga",
    "fb.load.fuel_used": "Combustible usado",
    "fb.aircraft.type": "Tipo",
    "fb.aircraft.registration": "Matrícula",
    "fb.aircraft.airline": "Aerolínea",
    "fb.aircraft.title": "Título",
    "fb.delete.title": "¿Eliminar vuelo?",
    "fb.delete.body":
      "Esta acción no se puede deshacer. El vuelo, su traza completa y todas las ediciones se borrarán para siempre.",
    "fb.delete.confirm": "Eliminar",
    "fb.action.edit": "Editar",
    "fb.action.deselect": "Deseleccionar",
    "fb.card.select_tooltip":
      "Click para ver la ruta real de este vuelo en el mapa",
    "fb.card.no_route": "Ruta sin datos",
  },
  en: {
    "common.cancel": "Cancel",
    "common.save": "Save",
    "common.close": "Close",
    "common.delete": "Delete",
    "common.confirm": "Confirm",
    "common.loading": "Loading…",
    "common.error": "Error",
    "common.errors": "errors",
    "common.search": "Search",
    "common.restart": "Restart",
    "common.previous": "← Previous",
    "common.next": "Next →",
    "common.page": "Page",
    "common.ready": "Ready",
    "common.open": "Open",
    "common.refresh": "Refresh",
    "common.install": "Install",
    "common.skip": "Skip",
    "common.got_it": "Got it",
    "common.unknown": "unknown",

    "nav.dashboard": "Dashboard",
    "nav.search": "Search",
    "nav.map": "Map (scenery)",
    "nav.addons": "Addons",
    "nav.flightbook": "FlightBook",

    "header.tagline": "FlightBook · SimBrief · SimConnect · GSX",
    "header.settings.tooltip": "Settings",

    "search.placeholder.scenery":
      "Search by airport, ICAO or developer…",
    "search.placeholder.simplaza":
      "Search by aircraft, livery, mod or author…",

    "demo.title":
      "Demo mode (browser). Install Rust and run {cmd} to enable real search.",

    "splash.task.update_check": "Check for app update",
    "splash.task.sources": "Load sources",
    "splash.task.settings": "Load settings",
    "splash.task.simbrief": "Load SimBrief",
    "splash.task.flightlog": "Load Flight Log",
    "splash.task.downloads": "Subscribe to downloads",
    "splash.task.scan_community": "Scan Community folder",
    "splash.task.refresh_updates": "Check addon updates",
    "splash.task.preload_catalogs": "Pre-load catalogs",

    "splash.tagline.0": "Preparing addon catalog…",
    "splash.tagline.1": "Syncing Community folder…",
    "splash.tagline.2": "Loading world airports…",
    "splash.tagline.3": "Warming up engines…",
    "splash.tagline.4": "Checking latest versions…",
    "splash.tagline.5": "Drawing routes on the map…",

    "splash.finishing": "Finishing",

    "update.banner.new_version": "New version available: v{version}",
    "update.banner.body":
      "You're on v{current}. We recommend updating before continuing.",
    "update.banner.view_changes": "View changes in this release",
    "update.banner.see_full_release": "(see full release)",
    "update.banner.auto_restart": "Restart the app automatically when done",
    "update.banner.install_now": "Install now",
    "update.progress.downloading": "Downloading v{version}",
    "update.progress.downloading_pct": "Downloading v{version} · {pct}%",
    "update.progress.complete":
      "Download complete — launching the installer. The app will close in a few seconds.",

    "settings.title": "Settings",
    "settings.section.general": "General",
    "settings.section.flights": "Flights",
    "settings.section.map_display": "Map display (FlightBook)",
    "settings.section.folders": "Folders",
    "settings.section.backup": "Backup",
    "settings.section.gsx": "GSX Pro",
    "settings.section.cloud": "Google Drive sync",
    "settings.section.import": "Import inventory",
    "settings.section.export": "Export inventory",
    "settings.section.storage": "Storage",
    "settings.section.tour": "Welcome tour",
    "settings.section.about": "About",
    "settings.section.appearance": "Appearance",
    "settings.section.behavior": "Behavior",
    "settings.section.language": "Language",
    "settings.section.simbrief": "SimBrief",

    "settings.language.title": "Language · Idioma",
    "settings.language.description":
      "\"Auto\" uses the operating system language. Manual changes require a restart to fully apply.",
    "settings.language.auto": "Auto (system)",
    "settings.language.es": "Español",
    "settings.language.en": "English",

    "settings.theme.title": "Visual theme",
    "settings.theme.description":
      "Dark or light. Light replaces backgrounds, cards and text while keeping accent colors. The FlightBook globe always uses satellite tiles.",
    "settings.theme.dark": "Dark",
    "settings.theme.light": "Light",

    "settings.autostart.title": "Start with Windows",
    "settings.autostart.hint":
      "The app launches automatically when you sign in.",
    "settings.updates.title": "Check updates at startup",
    "settings.updates.hint":
      "Looks for new app versions on GitHub when launching.",
    "settings.tray.title": "Minimize to tray on close",
    "settings.tray.hint":
      "Pressing X hides the app to the system tray. Right-click the icon → Quit to exit.",

    "restart.title": "Restart SimFleet",
    "restart.body":
      "The language was updated. To make all texts change (including modules loaded before the change), close and reopen SimFleet.",
    "restart.button": "Got it",

    "flying.preflight": "Pre-flight",
    "flying.taxi_out": "Taxi out",
    "flying.takeoff": "Takeoff",
    "flying.climbing": "Climbing",
    "flying.cruise": "Cruise",
    "flying.descent": "Descent",
    "flying.approach": "Approach",
    "flying.landed_rollout": "Landing",
    "flying.taxi_in": "Taxi in",
    "flying.parking": "At gate",
    "flying.deboarding": "Deboarding",
    "flying.pushback": "Pushback",
    "flying.engine_running": "Engines on",

    // FlightBook
    "fb.title": "FlightBook",
    "fb.subtitle.list": "Logbook of real flights.",
    "fb.subtitle.detail": "Flight detail — real route recorded every 10s.",
    "fb.back_to_globe": "Back to globe",
    "fb.back_to_globe.tooltip":
      "Return to the globe view with all flights",
    "fb.empty.title": "No flights logged yet",
    "fb.empty.body":
      "When you take off in MSFS with SimConnect active, the watcher will create an entry automatically.",
    "fb.flying_now": "Flying now",
    "fb.selected_flight": "Selected flight",
    "fb.real_route": "Real route (every 10s)",
    "fb.history": "History",
    "fb.peak.altitude": "Peak altitude",
    "fb.peak.speed": "Peak speed",
    "fb.stat.flights": "Flights",
    "fb.stat.total_time": "Total time",
    "fb.stat.distance": "Distance",
    "fb.stat.passengers": "Passengers",
    "fb.stat.cargo": "Cargo",
    "fb.stat.total_fuel": "Total fuel",
    "fb.stat.flights.suffix": "Block-to-block recorded",
    "fb.stat.time.suffix": "Block-to-block summed",
    "fb.block.route": "Route",
    "fb.block.times": "Times",
    "fb.block.load": "Load (SimBrief)",
    "fb.block.aircraft": "Aircraft",
    "fb.block.peak": "Peak metrics",
    "fb.route.origin": "Origin",
    "fb.route.destination": "Destination",
    "fb.route.alternate": "Alternate",
    "fb.route.distance": "Distance",
    "fb.route.dep_gate": "Dep gate",
    "fb.route.arr_gate": "Arr gate",
    "fb.times.out": "OUT (block-out)",
    "fb.times.off": "OFF (takeoff)",
    "fb.times.on": "ON (touchdown)",
    "fb.times.in": "IN (block-in)",
    "fb.times.block": "Block",
    "fb.times.flight": "Flight (air)",
    "fb.times.paused": "Paused",
    "fb.load.passengers": "Passengers",
    "fb.load.cargo": "Cargo",
    "fb.load.fuel_used": "Fuel used",
    "fb.aircraft.type": "Type",
    "fb.aircraft.registration": "Registration",
    "fb.aircraft.airline": "Airline",
    "fb.aircraft.title": "Title",
    "fb.delete.title": "Delete flight?",
    "fb.delete.body":
      "This action cannot be undone. The flight, full track and all edits will be permanently removed.",
    "fb.delete.confirm": "Delete",
    "fb.action.edit": "Edit",
    "fb.action.deselect": "Deselect",
    "fb.card.select_tooltip":
      "Click to view this flight's actual track on the map",
    "fb.card.no_route": "No route data",
  },
};

/** Resuelve `"auto"` contra `navigator.language` ("es-ES" → "es",
 *  "en-US" → "en", anything else → "en"). */
export function resolveLocale(setting: LocaleSetting): Locale {
  if (setting === "es" || setting === "en") return setting;
  if (typeof navigator !== "undefined") {
    const lang = navigator.language.toLowerCase();
    if (lang.startsWith("es")) return "es";
  }
  return "en";
}

let currentLocale: Locale = "en";

export function setActiveLocale(setting: LocaleSetting): Locale {
  currentLocale = resolveLocale(setting);
  return currentLocale;
}

export function getActiveLocale(): Locale {
  return currentLocale;
}

/**
 * (v3.4.0) Cargado SÍNCRONO del locale antes del primer render.
 *
 * Llamado desde `main.tsx` justo antes de `ReactDOM.createRoot`.
 * Lee `localStorage[LS_KEY]` — escrito por `persistLocale()` cada
 * vez que el usuario cambia idioma (también en el bootstrap del
 * store). Si no hay valor guardado, cae a `"auto"` y resuelve
 * contra `navigator.language`.
 *
 * **Por qué localStorage y no la DB:** la DB de Tauri es async; en
 * el momento en que React renderiza por primera vez el bootstrap
 * aún no terminó. Con localStorage tenemos lectura síncrona y
 * el shell entero (tabs, splash) ya arranca en el idioma correcto.
 */
export function preloadLocale(): Locale {
  let stored: string | null = null;
  try {
    stored =
      typeof localStorage !== "undefined" ? localStorage.getItem(LS_KEY) : null;
  } catch {
    // SecurityError en algunos contextos (iframe sandbox, etc).
    stored = null;
  }
  const setting: LocaleSetting =
    stored === "es" || stored === "en" || stored === "auto" ? stored : "auto";
  return setActiveLocale(setting);
}

/**
 * (v3.4.0) Espeja el setting de idioma al localStorage. Llamado por
 * la store cada vez que `setLanguage()` se ejecuta y al final del
 * bootstrap (para que reflectee lo que la DB tenga guardado, en
 * caso de que el usuario instale en otro PC con sync de cloud).
 */
export function persistLocale(setting: LocaleSetting): void {
  try {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(LS_KEY, setting);
    }
  } catch {
    // Ignorar — el usuario seguirá viendo el locale resuelto en memoria
    // hasta el restart. La DB sigue siendo source of truth.
  }
}

/** Lookup con fallback EN → key. Acepta `args` opcional para
 *  interpolación simple `{name}` → value. */
export function t(key: string, args?: Record<string, string | number>): string {
  const dict = DICTIONARIES[currentLocale] ?? DICTIONARIES.en;
  let value = dict[key] ?? DICTIONARIES.en[key] ?? key;
  if (args) {
    for (const [k, v] of Object.entries(args)) {
      value = value.replace(`{${k}}`, String(v));
    }
  }
  return value;
}
