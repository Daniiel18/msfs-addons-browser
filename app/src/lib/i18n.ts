/**
 * (v3.1.0) Sistema i18n minimal sin dependencias externas.
 *
 * Estrategia:
 *   · Diccionarios estáticos ES/EN. Las claves son strings cortos
 *     legibles tipo dot.notation ("settings.theme.title").
 *   · `t(key)` hace lookup en el dict del idioma activo; si no
 *     encuentra la clave, cae a EN; si tampoco, devuelve la clave
 *     cruda como último recurso.
 *   · El idioma activo vive en `useSettingsStore.settings.language`.
 *     Default = "auto" — al primer arranque resolvemos contra
 *     `navigator.language` (que en Tauri 2 refleja la locale del SO).
 *   · El cambio de idioma se persiste en settings y dispara un modal
 *     "reinicia para aplicar cambios completos" — no recargamos la UI
 *     en caliente porque muchos strings están hardcodeados todavía
 *     y un reload garantiza consistencia.
 *
 * Cobertura inicial: Settings + selector. El resto de la app sigue en
 * ES por defecto; añadir traducciones es trivial extendiendo los
 * dicts. La intención es ir migrando módulos a `t()` iterativamente.
 */

export type Locale = "es" | "en";
export type LocaleSetting = Locale | "auto";

const DICTIONARIES: Record<Locale, Record<string, string>> = {
  es: {
    "common.cancel": "Cancelar",
    "common.save": "Guardar",
    "common.close": "Cerrar",
    "common.delete": "Eliminar",
    "common.confirm": "Confirmar",
    "common.loading": "Cargando…",
    "common.error": "Error",
    "common.search": "Buscar",
    "common.restart": "Reiniciar",

    "nav.dashboard": "Dashboard",
    "nav.search": "Buscar",
    "nav.map": "Mapa (escenarios)",
    "nav.addons": "Addons",
    "nav.flightbook": "FlightBook",

    "settings.title": "Configuración",
    "settings.section.appearance": "Apariencia",
    "settings.section.behavior": "Comportamiento",
    "settings.section.language": "Idioma",
    "settings.section.simbrief": "SimBrief",
    "settings.section.cloud": "Sincronización en la nube",
    "settings.section.about": "Acerca de",

    "settings.language.title": "Idioma",
    "settings.language.description":
      "Selecciona el idioma de la interfaz. «Auto» usa el idioma del sistema operativo.",
    "settings.language.auto": "Auto (sistema)",
    "settings.language.es": "Español",
    "settings.language.en": "English",

    "settings.theme.title": "Tema",
    "settings.theme.dark": "Oscuro",
    "settings.theme.light": "Claro",

    "restart.title": "Reinicia SimFleet",
    "restart.body":
      "Cambiar el idioma requiere reiniciar la app para aplicar todos los textos correctamente.",
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

    // FlightBook (v3.3.0)
    "fb.title": "FlightBook",
    "fb.empty.title": "Aún no hay vuelos registrados",
    "fb.empty.body":
      "Cuando despegues en MSFS con SimConnect activo, el watcher creará una entrada automáticamente.",
    "fb.flying_now": "Volando ahora",
    "fb.selected_flight": "Vuelo seleccionado",
    "fb.stat.flights": "Vuelos",
    "fb.stat.total_time": "Tiempo total",
    "fb.stat.distance": "Distancia",
    "fb.stat.passengers": "Pasajeros",
    "fb.stat.cargo": "Carga",
    "fb.stat.total_fuel": "Combustible total",
    "fb.block.route": "Ruta",
    "fb.block.times": "Tiempos",
    "fb.block.load": "Carga (SimBrief)",
    "fb.block.aircraft": "Aeronave",
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
  },
  en: {
    "common.cancel": "Cancel",
    "common.save": "Save",
    "common.close": "Close",
    "common.delete": "Delete",
    "common.confirm": "Confirm",
    "common.loading": "Loading…",
    "common.error": "Error",
    "common.search": "Search",
    "common.restart": "Restart",

    "nav.dashboard": "Dashboard",
    "nav.search": "Search",
    "nav.map": "Map (scenery)",
    "nav.addons": "Addons",
    "nav.flightbook": "FlightBook",

    "settings.title": "Settings",
    "settings.section.appearance": "Appearance",
    "settings.section.behavior": "Behavior",
    "settings.section.language": "Language",
    "settings.section.simbrief": "SimBrief",
    "settings.section.cloud": "Cloud sync",
    "settings.section.about": "About",

    "settings.language.title": "Language",
    "settings.language.description":
      "Choose the interface language. \"Auto\" uses the operating system language.",
    "settings.language.auto": "Auto (system)",
    "settings.language.es": "Español",
    "settings.language.en": "English",

    "settings.theme.title": "Theme",
    "settings.theme.dark": "Dark",
    "settings.theme.light": "Light",

    "restart.title": "Restart SimFleet",
    "restart.body":
      "Changing the language requires restarting the app to apply all texts correctly.",
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

    // FlightBook (v3.3.0)
    "fb.title": "FlightBook",
    "fb.empty.title": "No flights logged yet",
    "fb.empty.body":
      "When you take off in MSFS with SimConnect active, the watcher will create an entry automatically.",
    "fb.flying_now": "Flying now",
    "fb.selected_flight": "Selected flight",
    "fb.stat.flights": "Flights",
    "fb.stat.total_time": "Total time",
    "fb.stat.distance": "Distance",
    "fb.stat.passengers": "Passengers",
    "fb.stat.cargo": "Cargo",
    "fb.stat.total_fuel": "Total fuel",
    "fb.block.route": "Route",
    "fb.block.times": "Times",
    "fb.block.load": "Load (SimBrief)",
    "fb.block.aircraft": "Aircraft",
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
