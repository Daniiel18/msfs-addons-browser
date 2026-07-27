import { useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { getActiveLocale, t } from "../lib/i18n";
import {
  AlertCircle,
  ArrowLeft,
  ClipboardCheck,
  Clock,
  Droplet,
  Globe,
  MapPin,
  Package,
  Plane,
  PlaneLanding,
  Ruler,
  Search,
  Share2,
  Swords,
  Trash2,
  Users,
} from "lucide-react";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useAppStore } from "../stores/useAppStore";
import { findVsSnapshot } from "../stores/useLiveVsStore";
import { useUnits } from "../lib/units";
import type { AirportBrief, AirlineTag, FlightLogEntry } from "../lib/types";
import { RoutesMapView } from "./RoutesMapView";
import { ShareCardModal } from "./ShareCardModal";
import { buildFlightCardSvg } from "../lib/shareCards";
import { buildFlightBookHtml, saveFlightBookHtml } from "../lib/flightbookExport";
import { useToastStore } from "../stores/useToastStore";
import { EditFlightModal } from "./EditFlightModal";
import { VsCombatModal } from "./VsCombatModal";
import { DamageBadge } from "./DamageBadge";
import { AirlineLogo } from "./AirlineLogo";
import { icaoToName, canonicalAirlineKey } from "../lib/airlineCodes";
import { Pencil, Minimize2, Maximize2, ChevronLeft, ChevronRight } from "lucide-react";

// (v4.24.2) Persistencia del colapso del sidebar de vuelos. El
// FlightBook se DESMONTA al cambiar de pestaña (App.tsx lo renderiza
// condicionalmente), así que un useState plano volvía al default y el
// usuario perdía el estado que había elegido. Espejamos a localStorage
// (lectura síncrona, mismo patrón que i18n/units) para que quede
// exactamente como lo dejó hasta que él mismo lo cambie.
const SIDEBAR_COLLAPSED_KEY = "simfleet.fb.sidebarCollapsed";
// (v4.27.0) Misma idea para el vuelo seleccionado — al volver de
// otra pestaña, el panel y el track del vuelo deben seguir visibles
// hasta que el usuario lo deseleccione explícitamente.
const SELECTED_FLIGHT_KEY = "simfleet.fb.selectedFlightId";

/** (v6.2.35) Limpia el nombre del avión para mostrar: algunas liveries
 *  cuelan " AEROLÍNEA (MATRÍCULA)" en el título ("PMDG 737-800 DAL
 *  (N374DA)"). Quitamos ese sufijo para que quede sólo el avión. No toca
 *  los nombres limpios ("Airbus", "Boeing 737-800"). */
export function cleanAircraftLabel(s: string | null | undefined): string {
  if (!s) return "";
  return s
    .replace(/\s+[A-Z0-9]{2,4}\s*\([A-Za-z0-9-]{2,10}\)\s*$/, "")
    .replace(/\s*\([A-Za-z0-9-]{2,10}\)\s*$/, "")
    .trim();
}

function readSidebarCollapsed(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

function readSelectedFlightId(): number | null {
  try {
    const raw = localStorage.getItem(SELECTED_FLIGHT_KEY);
    if (!raw) return null;
    const n = Number(raw);
    return Number.isFinite(n) ? n : null;
  } catch {
    return null;
  }
}

/**
 * Vista completa del FlightBook — bitácora de vuelos reales
 * capturados por SimConnect.
 *
 * Tres niveles:
 *   1. **Resumen** arriba — totales y agregados.
 *   2. **Vuelo activo** (si lo hay) destacado con badge verde.
 *   3. **Tabla de historial** — un row por vuelo con todos los
 *      campos: ICAOs, gates, FPM, max speed, distancia, duración.
 *
 * El watcher (`simconnect_watcher`) inserta una fila al detectar
 * despegue y la actualiza al detectar aterrizaje (con todas las
 * métricas calculadas en `handle_aircraft_data`).
 */
export function FlightBookView() {
  const entries = useFlightLogStore((s) => s.entries);
  const lastError = useFlightLogStore((s) => s.lastError);
  const reload = useFlightLogStore((s) => s.reload);
  const remove = useFlightLogStore((s) => s.remove);
  // (v3.4.10) Status del watcher para mostrar PreflightCard cuando
  // hay SimConnect conectado pero aún no hay vuelo abierto en DB
  // (cold & dark spawn at gate).
  const status = useFlightLogStore((s) => s.status);

  // Selección de vuelo para mostrar su track real en el mapa.
  // null/undefined = modo globo (todos los vuelos a la vez).
  // (v4.27.0) Inicializado desde localStorage y espejado en cada
  // cambio: sobrevive a remounts (cambio de pestaña) y reinicios de
  // la app, mismo patrón que el colapso del sidebar.
  const [selectedFlightId, setSelectedFlightIdState] = useState<number | null>(
    readSelectedFlightId,
  );
  const setSelectedFlightId: typeof setSelectedFlightIdState = (value) => {
    setSelectedFlightIdState(value);
    try {
      const next = typeof value === "function" ? value(selectedFlightId) : value;
      if (next == null) localStorage.removeItem(SELECTED_FLIGHT_KEY);
      else localStorage.setItem(SELECTED_FLIGHT_KEY, String(next));
    } catch {
      // localStorage no disponible — el estado en memoria sigue ok.
    }
  };
  // (v3.2.0) Modal de confirmación de borrado — reemplaza window.confirm
  // por una UI estética. `null` = oculto. Guardamos el entry completo
  // para mostrar contexto (orig→dest + fecha) y el id para llamar
  // remove() al confirmar.
  const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);
  // (v3.5.0) Filtro de búsqueda del sidebar — matchea ICAO origen,
  // destino, aerolínea, registration, ATC type. Vacío = todos.
  const [sidebarQuery, setSidebarQuery] = useState("");
  // (v4.18.0) Colapso lateral del sidebar de vuelos — el mapa toma todo
  // el ancho. Botón chevron en la barra del filtro / franja de reapertura.
  // (v4.24.2) Inicializa desde localStorage y espeja cada cambio: el
  // estado sobrevive a cambios de pestaña y a reinicios de la app.
  const [sidebarCollapsed, setSidebarCollapsedState] = useState(readSidebarCollapsed);
  const setSidebarCollapsed = (v: boolean) => {
    setSidebarCollapsedState(v);
    try {
      localStorage.setItem(SIDEBAR_COLLAPSED_KEY, v ? "1" : "0");
    } catch {
      // localStorage lleno/deshabilitado — el estado en memoria sigue ok.
    }
  };
  // (v3.6.0 Phase H — Epic E) Estado de visibilidad del ChecklistWidget.
  const [checklistOpen, setChecklistOpen] = useState(false);
  // (v6 #3) Estado del modal de combate Live VS.
  const [vsOpen, setVsOpen] = useState(false);
  // (v3.6.1 fix I6) Colapso del SelectedFlightPanel. Cuando true,
  // sólo se muestra el header con la ruta y el grade — el resto del
  // mapa queda visible para que el usuario pueda ver la trayectoria
  // del vuelo seleccionado.
  const [panelCollapsed, setPanelCollapsed] = useState(false);
  const [showShare, setShowShare] = useState(false);
  const [exportingWeb, setExportingWeb] = useState(false);
  const pushExportToast = useToastStore((s) => s.push);

  // (v6.2.31 / R8) Exporta el FlightBook a un HTML autocontenido de
  // solo lectura para compartir.
  const exportWeb = async () => {
    if (exportingWeb) return;
    setExportingWeb(true);
    try {
      const { api } = await import("../lib/tauri");
      const pilot = await api.pilotProfile().catch(() => null);
      const locale = getActiveLocale() === "es" ? "es-ES" : "en-US";
      const html = buildFlightBookHtml(
        entries,
        pilot?.name ?? "",
        {
          title: t("fbexport.title"),
          subtitle: t("fbexport.subtitle"),
          flights: t("wrapped.flights"),
          hours: t("wrapped.hours"),
          distance: t("wrapped.distance"),
          airports: t("wrapped.airports"),
          colDate: t("fbexport.col_date"),
          colRoute: t("fbexport.col_route"),
          colAircraft: t("fbexport.col_aircraft"),
          colDistance: t("fbexport.col_distance"),
          colDuration: t("fbexport.col_duration"),
          colLanding: t("fbexport.col_landing"),
          footer: t("fbexport.footer"),
          routeMap: t("fbexport.route_map"),
        },
        locale,
      );
      const path = await saveFlightBookHtml(html, "simfleet-flightbook.html");
      if (path) {
        pushExportToast({
          kind: "success",
          title: t("fbexport.saved"),
          message: path,
          ttlMs: 3000,
        });
      }
    } catch (e) {
      pushExportToast({ kind: "error", title: t("fbexport.error"), message: String(e) });
    } finally {
      setExportingWeb(false);
    }
  };

  const selectedFlight = useMemo(
    () =>
      selectedFlightId != null
        ? entries.find((e) => e.id === selectedFlightId) ?? null
        : null,
    [entries, selectedFlightId],
  );

  // (v6.2.13) ¿El vuelo seleccionado tiene un Crew VS guardado? El botón se
  // habilita si hay CUALQUIER dato del duelo (tú o el rival) — ya no exige que
  // Héctor estuviese conectado. El snapshot se busca tolerante a la fecha
  // (día ±1) porque el canal en vivo y el histórico pueden diferir por un día.
  const vsSnapshotExists = useMemo(() => {
    const f = selectedFlight;
    if (!f?.originIcao || !f?.destinationIcao || !f?.startedAt) return false;
    const snap = findVsSnapshot(
      f.startedAt.slice(0, 10),
      f.originIcao,
      f.destinationIcao,
    );
    return snap != null && (snap.self != null || snap.rival != null);
  }, [selectedFlight]);

  // Refresh al montar — el watcher emite eventos que actualizan
  // automáticamente, pero el primer mount necesita un pull.
  useEffect(() => {
    void reload();
  }, [reload]);

  // (v6.2.35) La campanita puede pedir abrir DIRECTO el Crew VS de un
  // vuelo (aviso "Crew VS listo"). Buscamos el vuelo que casa
  // origen/destino con la fecha más cercana al duelo, lo seleccionamos y
  // abrimos el modal de combate. Esperamos a que los vuelos estén
  // cargados antes de consumir (y limpiar) la petición.
  const crewVsRequest = useAppStore((s) => s.crewVsRequest);
  const clearCrewVsRequest = useAppStore((s) => s.clearCrewVsRequest);
  useEffect(() => {
    if (!crewVsRequest || entries.length === 0) return;
    const { origin, dest, at } = crewVsRequest;
    const matches = entries.filter(
      (e) =>
        (e.originIcao ?? "").toUpperCase() === origin.toUpperCase() &&
        (e.destinationIcao ?? "").toUpperCase() === dest.toUpperCase(),
    );
    if (matches.length > 0) {
      const best = matches.reduce((a, b) =>
        Math.abs(new Date(a.startedAt).getTime() - at) <=
        Math.abs(new Date(b.startedAt).getTime() - at)
          ? a
          : b,
      );
      setSelectedFlightId(best.id);
      setVsOpen(true);
    }
    clearCrewVsRequest();
  }, [crewVsRequest, entries, clearCrewVsRequest]);

  // Si el vuelo seleccionado desaparece (lo borraron, recarga falló),
  // volvemos al modo globo en lugar de quedar con un id roto.
  useEffect(() => {
    if (selectedFlightId != null && !entries.find((e) => e.id === selectedFlightId)) {
      setSelectedFlightId(null);
    }
  }, [entries, selectedFlightId]);

  // (v3.6.0 Phase H) Cierra el ChecklistWidget al volver al globo.
  // (v4.0.0 — P3.2) También al cambiar a un vuelo VAS import: el
  // botón Checklist se oculta para imports, así que el widget no
  // debe quedar abierto del vuelo simconnect anterior.
  useEffect(() => {
    if (!checklistOpen) return;
    const isSimflown = selectedFlight?.source === "simconnect";
    if (selectedFlightId == null || !isSimflown) {
      setChecklistOpen(false);
    }
  }, [selectedFlightId, selectedFlight?.source, checklistOpen]);

  // (v3.6.0 Phase H — Epic D) Tag de aerolínea activa (del store).
  // Lo declaramos ACÁ ARRIBA (antes que useEffects que lo consumen)
  // para evitar TDZ errors.
  const selectedAirline = useFlightLogStore((s) => s.selectedAirline);

  // (v3.6.4 fix K2) Cuando el usuario cambia de tag de aerolínea,
  // deseleccionar el vuelo actual. El usuario reportó: "el focus
  // cuando filtro por tags no funciona, solo funciona si hago un
  // cambio de pantallas". Causa: con un vuelo seleccionado el
  // RoutesMapView está en detailMode (oculta TODAS las rutas y
  // muestra sólo el track del seleccionado), así que el filter de
  // aerolínea no tenía efecto visual. Al cambiar de pantalla, el
  // vuelo se deselecciona y el map vuelve al modo overview donde
  // el filter sí aplica. Ahora hacemos esa transición automática.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (selectedAirline) {
      setSelectedFlightId(null);
    }
  }, [selectedAirline]);

  // (v3.5.0 F3) `inFlight` solo cuenta cuando el watcher de SimConnect
  // está REALMENTE conectado y reportando posición. Sin esto, un vuelo
  // descargado del cloud con `endedAt = null` (estaba en curso en otra
  // máquina al momento del sync) hacía aparecer un "FLYING NOW" fantasma.
  // El backend ya cierra estos phantoms al boot/post-sync, pero esta
  // guardia previene cualquier flash transitorio.
  // (v6.2.2) El vuelo ACTIVO en vivo es el abierto (endedAt null) mientras el
  // watcher esté conectado y reportando posición — ese sigue mostrándose como
  // "FLYING NOW" incluso en el taxi tras aterrizar (no hay regresión).
  const liveOpenId =
    status?.simconnectConnected && status.currentLat != null
      ? (entries.find((e) => e.endedAt === null)?.id ?? null)
      : null;
  const inFlight =
    liveOpenId != null ? entries.find((e) => e.id === liveOpenId) : undefined;
  // (v6.2.2) Un vuelo con `landingFpm` YA aterrizó: es "completado" aunque su
  // `endedAt` siga null — pasa si el piloto sale sin apagar motores y no se
  // dispara el cierre por ENGINE SHUTDOWN. El backend lo finaliza al cerrar la
  // sesión del sim, pero esta guardia lo muestra YA (sin reiniciar) salvo que
  // sea justamente el vuelo activo en vivo (taxi post-aterrizaje).
  const completed = entries.filter(
    (e) => e.endedAt !== null || (e.landingFpm != null && e.id !== liveOpenId),
  );

  // `selectedAirline` ya declarado arriba (antes del useEffect del fix K2).

  // (v3.6.0 Phase H — Epic D) Pool de vuelos para stats — si hay
  // airline activa, usamos sólo esos; sino, todos los completados.
  // Esto hace que el StatsWidget muestre KPIs específicos al filtro.
  const statsPool = useMemo(() => {
    if (!selectedAirline) return completed;
    const selKey = canonicalAirlineKey(selectedAirline.icao, selectedAirline.name);
    return completed.filter(
      (e) => canonicalAirlineKey(e.airlineIcao, e.aircraftAirline) === selKey,
    );
  }, [completed, selectedAirline]);
  const stats = useMemo(() => {
    if (statsPool.length === 0) {
      return null;
    }
    const totalSec = statsPool.reduce(
      (acc, e) => acc + (e.flightTimeS ?? 0),
      0,
    );
    const totalDistance = statsPool.reduce(
      (acc, e) => acc + (e.distanceNm ?? 0),
      0,
    );
    // (v2.2.0) FPM removido — la captura no era precisa.
    // v1.0.0: totales para los nuevos stat-cards de Pasajeros / Carga / Fuel.
    // Filtramos valores null antes de sumar — un vuelo viejo sin estos
    // campos no tira el total a NaN ni contamina la cuenta "X vuelos
    // con datos". Si NINGÚN vuelo reporta, mostramos "—" en el card.
    const passengersValid = statsPool
      .map((e) => e.passengers)
      .filter((v): v is number => v !== null);
    const cargoValid = statsPool
      .map((e) => e.cargoKg)
      .filter((v): v is number => v !== null);
    const fuelValid = statsPool
      .map((e) => e.fuelUsedKg)
      .filter((v): v is number => v !== null);
    const totalPassengers = passengersValid.reduce((a, b) => a + b, 0);
    const totalCargoKg = cargoValid.reduce((a, b) => a + b, 0);
    const totalFuelKg = fuelValid.reduce((a, b) => a + b, 0);

    // (v3.5.0 F2 v4) Avg landing FPM — solo entre vuelos que CAPTURARON
    // el VS al touchdown (sim-flown + VAS-ACARS). MSFS logbook imports
    // no traen este dato → quedan filtrados naturalmente al filtrar
    // landingFpm !== null. El usuario pidió que se muestre como avg
    // en el resumen.
    const fpmValid = statsPool
      .map((e) => e.landingFpm)
      .filter((v): v is number => v !== null && v < 0);
    const avgLandingFpm =
      fpmValid.length > 0
        ? Math.round(fpmValid.reduce((a, b) => a + b, 0) / fpmValid.length)
        : null;

    return {
      count: statsPool.length,
      totalSec,
      totalDistance,
      totalPassengers,
      totalCargoKg,
      totalFuelKg,
      passengersFlightCount: passengersValid.length,
      cargoFlightCount: cargoValid.length,
      fuelFlightCount: fuelValid.length,
      avgLandingFpm,
      fpmFlightCount: fpmValid.length,
    };
  }, [statsPool]);

  // (v3.6.0 Phase H — Epic C/D) Sidebar list filtering — match against
  // ICAO orig/dest, airline, registration, aircraft type, **flight_number**,
  // **callsign**, **airline_icao**. Case-insensitive substring.
  // ADEMÁS aplica el filtro de aerolínea seleccionada del store (chip
  // arriba del mapa). Si selectedAirline está activo, sólo entran vuelos
  // que matchean el icao o el name fallback. `selectedAirline` viene
  // del declaration de arriba (compartido con statsPool).
  const filteredCompleted = useMemo(() => {
    let arr = completed;
    if (selectedAirline) {
      const selKey = canonicalAirlineKey(selectedAirline.icao, selectedAirline.name);
      arr = arr.filter(
        (e) => canonicalAirlineKey(e.airlineIcao, e.aircraftAirline) === selKey,
      );
    }
    if (!sidebarQuery.trim()) return arr;
    const q = sidebarQuery.trim().toLowerCase();
    return arr.filter((e) => {
      const haystack = [
        e.originIcao,
        e.destinationIcao,
        e.aircraftAtcType,
        e.aircraftAirline,
        e.aircraftRegistration,
        e.aircraftTitle,
        e.flightNumber,
        e.callsign,
        e.airlineIcao,
      ]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
      return haystack.includes(q);
    });
  }, [completed, sidebarQuery, selectedAirline]);

  // (v3.5.0 → v4.19.0) Pieza central del overlay flotante. Con
  // SimConnect conectado (preflight, vuelo activo, llegada) NO se
  // muestra ningún card de estado — feedback usuario: ahí solo va el
  // detalle de un vuelo viejo al clickearlo (el LiveFlightPanel del
  // mapa ya cubre el vuelo en curso). El Overview de stats se mantiene
  // únicamente cuando el sim no está conectado.
  const overlayKind: "selected" | "stats" | "none" = selectedFlight
    ? "selected"
    : inFlight || status?.simconnectConnected
      ? "none"
      : completed.length > 0
        ? "stats"
        : "none";

  return (
    <section className="flex h-[calc(100vh-13rem)] flex-col gap-3 overflow-hidden">
      <header className="flex shrink-0 items-center gap-3">
        <div className="min-w-0 shrink-0">
          <h1 className="flex items-center gap-2 text-lg font-semibold text-slate-100">
            <Plane className="h-5 w-5 text-emerald-300" />
            {t("fb.title")}
            {selectedFlight && (
              <span className="ml-1 inline-flex items-center gap-1 rounded-md bg-amber-500/15 px-2 py-0.5 text-xs font-medium text-amber-200 ring-1 ring-amber-500/30">
                <PlaneLanding className="h-3 w-3" />
                {selectedFlight.originIcao ?? "?"} →{" "}
                {selectedFlight.destinationIcao ?? "?"}
              </span>
            )}
          </h1>
          <p className="text-xs text-slate-500">
            {selectedFlight
              ? t("fb.subtitle.detail")
              : t("fb.subtitle.list")}
          </p>
        </div>
        {/* (v3.32.0) Tags de aerolíneas INLINE en el header, flex-1:
            ocupan el hueco entre el título y el botón "Volver al globo" y
            se EXPANDEN a todo el ancho libre cuando ese botón no está (sin
            vuelo seleccionado). Antes vivían en una fila aparte que robaba
            altura → subir el grid (sidebar+mapa) elimina el scroll de la
            pantalla. Se autoocultan si no hay aerolíneas. min-w-0 deja que
            su propio scroll horizontal (#15) funcione sin empujar el botón. */}
        <div className="min-w-0 flex-1">
          <AirlineTagFilter />
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {!selectedFlight && entries.length > 0 ? (
            <button
              onClick={() => void exportWeb()}
              disabled={exportingWeb}
              className="inline-flex items-center gap-1 rounded-md border border-slate-700 bg-slate-900/60 px-2.5 py-1.5 text-xs font-medium text-slate-300 hover:border-brand-500/40 hover:text-slate-100 disabled:opacity-50"
              title={t("fbexport.tooltip")}
            >
              <Share2 className="h-3 w-3" />
              {t("fbexport.button")}
            </button>
          ) : null}
          {selectedFlight ? (
            <button
              onClick={() => setShowShare(true)}
              className="inline-flex items-center gap-1 rounded-md border border-brand-500/40 bg-brand-500/10 px-2.5 py-1.5 text-xs font-medium text-brand-200 hover:border-brand-400 hover:bg-brand-500/20"
              title={t("share.flight.tooltip")}
            >
              <Share2 className="h-3 w-3" />
              {t("share.flight.button")}
            </button>
          ) : null}
          {selectedFlight ? (
            <button
              onClick={() => setSelectedFlightId(null)}
              className="inline-flex items-center gap-1 rounded-md border border-amber-500/40 bg-amber-500/10 px-2.5 py-1.5 text-xs font-medium text-amber-200 hover:border-amber-400 hover:bg-amber-500/20"
              title={t("fb.back_to_globe.tooltip")}
            >
              <ArrowLeft className="h-3 w-3" />
              <Globe className="h-3 w-3" />
              {t("fb.back_to_globe")}
            </button>
          ) : null}
        </div>
      </header>

      {showShare && selectedFlight && (
        <ShareCardModal
          title={t("share.flight.title")}
          card={buildFlightCardSvg(
            selectedFlight,
            {
              brand: "SimFleet",
              distance: t("share.flight.distance"),
              duration: t("share.flight.duration"),
              ceiling: t("share.flight.ceiling"),
              landing: t("share.flight.landing"),
              aircraft: t("share.flight.aircraft"),
              tagline: t("share.flight.tagline"),
            },
            getActiveLocale() === "es" ? "es-ES" : "en-US",
          )}
          onClose={() => setShowShare(false)}
        />
      )}

      {lastError && (
        <div className="flex items-start gap-2 rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-xs text-rose-200">
          <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
          <span>{lastError}</span>
        </div>
      )}

      {/* (v3.5.0 — redesign minimalista) **Mapa como hero + glass overlay**:
          · Sidebar izquierda compacta (~300px): lista densificada con
            filtro, click selecciona vuelo.
          · Panel derecho: MAPA cubre toda la superficie (sin chrome).
            Sobre él flota una card glass con backdrop-blur que muestra
            el contenido relevante (vuelo seleccionado / activo /
            preflight / stats agregados / nada).
          Patrón visual a lo Flightradar24 / Polarsteps: el mapa es el
          contexto permanente, los datos son overlays semitransparentes
          que dejan ver el geografía detrás. */}
      <div
        className={`grid min-h-0 flex-1 grid-cols-1 gap-3 ${
          sidebarCollapsed
            ? "lg:grid-cols-[18px_minmax(0,1fr)]"
            : "lg:grid-cols-[18px_300px_minmax(0,1fr)]"
        }`}
      >
        {/* (v4.26.0) HANDLE de colapso pegado al borde izquierdo de la
            app — columna fina full-height SIEMPRE visible, mismo
            patrón que el Map (scenery). Reemplaza al chevron que vivía
            dentro de la barra del filtro + la franja de reapertura. */}
        <button
          onClick={() => setSidebarCollapsed(!sidebarCollapsed)}
          title={sidebarCollapsed ? t("fb.sidebar.expand") : t("fb.sidebar.collapse")}
          className="hidden items-center justify-center rounded-2xl border border-slate-800 bg-slate-900/40 text-slate-500 hover:bg-slate-800/60 hover:text-slate-200 lg:flex"
        >
          {sidebarCollapsed ? (
            <ChevronRight className="h-4 w-4" />
          ) : (
            <ChevronLeft className="h-4 w-4" />
          )}
        </button>
        {!sidebarCollapsed && (
        /* SIDEBAR — lista compacta de vuelos con filtro sticky arriba. */
        <aside className="flex min-h-0 flex-col rounded-2xl border border-slate-800 bg-slate-900/40">
          <div className="sticky top-0 z-10 flex items-center gap-2 rounded-t-2xl border-b border-slate-800 bg-slate-900/80 px-3 py-2 backdrop-blur">
            <Search className="h-3.5 w-3.5 shrink-0 text-slate-500" />
            <input
              value={sidebarQuery}
              onChange={(e) => setSidebarQuery(e.target.value)}
              placeholder={t("fb.sidebar.filter_placeholder")}
              className="w-full bg-transparent text-xs text-slate-100 placeholder:text-slate-600 focus:outline-none"
            />
            <span className="shrink-0 rounded bg-slate-800 px-1.5 py-0.5 text-[10px] tabular-nums text-slate-400">
              {filteredCompleted.length}
            </span>
          </div>
          <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
            {completed.length === 0 ? (
              <div className="px-3 py-8 text-center">
                <Plane className="mx-auto mb-2 h-6 w-6 text-slate-600" />
                <p className="text-xs font-medium text-slate-400">
                  {t("fb.empty.title")}
                </p>
                <p className="mt-1 text-[10px] text-slate-500">
                  {t("fb.empty.body")}
                </p>
              </div>
            ) : filteredCompleted.length === 0 ? (
              <div className="px-3 py-6 text-center text-[11px] text-slate-500">
                {t("fb.sidebar.no_matches")}
              </div>
            ) : (
              <ul className="space-y-1">
                {filteredCompleted.map((e) => (
                  <CompactFlightRow
                    key={e.id}
                    entry={e}
                    selected={e.id === selectedFlightId}
                    onSelect={() =>
                      setSelectedFlightId((prev) =>
                        prev === e.id ? null : e.id,
                      )
                    }
                    onDelete={() => setConfirmDeleteId(e.id)}
                  />
                ))}
              </ul>
            )}
          </div>
        </aside>
        )}

        {/* COLUMNA DERECHA — apila vertical (v3.6.3 reorganización):
              · AirlineTagFilter chips (siempre, FUERA del mapa).
              · DetailActionsBar (sólo cuando hay vuelo seleccionado).
              · Map canvas con glass overlays. */}
        <div className="flex min-h-0 flex-col gap-2">
          {/* (v3.26.0 P7.10) Veredicto de daños/forzado del vuelo. */}
          {selectedFlightId != null && (
            <DamageBadge flightId={selectedFlightId} />
          )}
          {/* (v3.6.3 fix J5) Tabs (Checklist / Weather / NOTAMs) SOLO
              con vuelo seleccionado. Antes siempre se renderizaban —
              el usuario no podía hacer nada útil con ellos sin un
              vuelo activo. */}
          {selectedFlightId != null && (
            <DetailActionsBar
              selectedFlightId={selectedFlightId}
              selectedFlightSource={selectedFlight?.source ?? null}
              onToggleChecklist={() => setChecklistOpen((v) => !v)}
              checklistOpen={checklistOpen && selectedFlightId != null}
              onToggleVs={() => setVsOpen((v) => !v)}
              vsOpen={vsOpen && selectedFlightId != null}
              vsHasSnapshot={vsSnapshotExists}
            />
          )}
          <div className="relative min-h-0 flex-1 overflow-hidden rounded-2xl border border-slate-800 bg-slate-950/40">
            <div className="absolute inset-0">
              <RoutesMapView
                height="100%"
                selectedFlightId={selectedFlightId}
              />
            </div>

            {/* (v3.6.0 Phase H — Epic E) ChecklistWidget — glass
                overlay con score breakdown del vuelo seleccionado.
                Se abre con el botón Checklist en DetailActionsBar. */}
            {checklistOpen && selectedFlightId != null && (
              <ChecklistWidget
                flightId={selectedFlightId}
                onClose={() => setChecklistOpen(false)}
              />
            )}


            {/* (v6 #3) Modal de combate Live VS — comparativa Daniel vs
                Héctor (avión, aerolínea, matrícula, foto, FPM en vivo). */}
            {vsOpen && selectedFlightId != null && (
              <VsCombatModal
                entry={selectedFlight}
                onClose={() => setVsOpen(false)}
              />
            )}

            {/* GLASS OVERLAY — card de detalle flotando sobre el mapa.
                · "selected": altura completa + scroll interno (el
                  contenido es largo: route + times + load + aircraft).
                · "active" / "preflight" / "stats": altura intrínseca
                  (sin scroll, contenido compacto). Framer `layout`
                  anima la transición de tamaño al volver al globo. */}
            <AnimatePresence>
              {overlayKind !== "none" && (
                <motion.div
                  key="overlay"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  transition={{ duration: 0.18 }}
                  className="pointer-events-none absolute inset-0 p-3 sm:p-4"
                >
                  <motion.div
                    layout
                    transition={{ duration: 0.28, ease: [0.4, 0, 0.2, 1] }}
                    className={`pointer-events-auto w-full ${
                      overlayKind === "stats" ? "max-w-[260px]" : "max-w-[420px]"
                    }`}
                    style={
                      overlayKind === "selected" && !panelCollapsed
                        ? { height: "100%" }
                        : undefined
                    }
                  >
                    <motion.div
                      layout
                      className={`overflow-hidden rounded-2xl border border-slate-700/70 bg-slate-950/75 shadow-2xl ring-1 ring-slate-800/70 backdrop-blur-xl ${
                        overlayKind === "selected" && !panelCollapsed
                          ? "flex h-full flex-col"
                          : ""
                      }`}
                    >
                      <div
                        className={`p-3 ${
                          overlayKind === "selected" && !panelCollapsed
                            ? "min-h-0 flex-1 overflow-y-auto"
                            : ""
                        }`}
                      >
                    {overlayKind === "selected" && selectedFlight && (
                      <SelectedFlightPanel
                        entry={selectedFlight}
                        collapsed={panelCollapsed}
                        onToggleCollapse={() => setPanelCollapsed((v) => !v)}
                      />
                    )}
                    {overlayKind === "stats" && stats && (
                      <StatsWidget stats={stats} />
                    )}
                      </div>
                    </motion.div>
                  </motion.div>
                </motion.div>
              )}
            </AnimatePresence>
          </div>
        </div>
      </div>

      {/* (v3.2.0) Modal de confirmación de borrado. Estético en vez
          del feo `window.confirm`. */}
      {confirmDeleteId != null && (() => {
        const target = entries.find((e) => e.id === confirmDeleteId);
        if (!target) {
          // Edge case: entry desapareció (refresh paralelo).
          setConfirmDeleteId(null);
          return null;
        }
        return (
          <DeleteFlightModal
            entry={target}
            onConfirm={() => {
              void remove(target.id);
              setConfirmDeleteId(null);
            }}
            onCancel={() => setConfirmDeleteId(null)}
          />
        );
      })()}
    </section>
  );
}

/** (v3.2.0) Modal estético de confirmación al borrar un vuelo del
 *  FlightBook. Reemplaza el `window.confirm` nativo (UI del SO) por
 *  un overlay con AnimatePresence + Tailwind, manteniendo el theme
 *  de la app. Botones rojos (Eliminar) y neutro (Cancelar). */
function DeleteFlightModal({
  entry,
  onConfirm,
  onCancel,
}: {
  entry: FlightLogEntry;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const orig = entry.originIcao ?? "?";
  const dest = entry.destinationIcao ?? "?";
  return (
    <div
      className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/75 backdrop-blur-sm"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-sm rounded-2xl border border-rose-500/30 bg-slate-900 shadow-2xl shadow-rose-500/10"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center gap-2 border-b border-slate-800 px-5 py-3">
          <div className="flex h-7 w-7 items-center justify-center rounded-full bg-rose-500/15 ring-1 ring-rose-500/30">
            <Trash2 className="h-3.5 w-3.5 text-rose-300" />
          </div>
          <h3 className="text-sm font-semibold text-slate-100">
            {t("fb.delete.title")}
          </h3>
        </header>
        <div className="space-y-3 px-5 py-4">
          <div className="rounded-md border border-slate-800 bg-slate-950/50 px-3 py-2">
            <div className="font-mono text-sm text-slate-100">
              {orig}
              <span className="mx-1.5 text-rose-300">→</span>
              {dest}
            </div>
            <div className="mt-0.5 text-[11px] text-slate-500">
              {formatDate(entry.startedAt)}
              {entry.aircraftAtcType && ` · ${entry.aircraftAtcType}`}
            </div>
          </div>
          <p className="text-xs text-slate-400">
            {t("fb.delete.body")}
          </p>
        </div>
        <footer className="flex justify-end gap-2 border-t border-slate-800 bg-slate-950/30 px-5 py-3">
          <button
            onClick={onCancel}
            className="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-600 hover:text-slate-100"
          >
            {t("common.cancel")}
          </button>
          <button
            onClick={onConfirm}
            className="inline-flex items-center gap-1 rounded-md bg-rose-500/80 px-3 py-1.5 text-xs font-semibold text-rose-50 hover:bg-rose-500"
          >
            <Trash2 className="h-3 w-3" />
            {t("fb.delete.confirm")}
          </button>
        </footer>
      </div>
    </div>
  );
}

/**
 * Panel de detalle del vuelo seleccionado. Reemplaza al StatGrid
 * agregado cuando el usuario clica un vuelo concreto — los stats
 * agregados (totales de todos los vuelos) son irrelevantes cuando
 * estás mirando UN vuelo específico, así que aquí ponemos sus
 * propias métricas: duración block-to-block, distancia, max alt,
 * max GS, landing FPM coloreado.
 */
/** (v3.3.0) Panel de detalle del vuelo seleccionado — rediseño
 *  completo con bloques organizados estilo "aviation log" moderno.
 *
 *  Estructura:
 *    · Header sticky: ruta XL + aerolínea/aeronave + acciones.
 *    · Bloque 1 — Route: orig, dest, alterno (si hay), distance, gates.
 *    · Bloque 2 — Times: OUT, IN, Block. (OFF/ON pendiente de captura
 *      en el watcher; mostramos lo que tenemos honestamente.)
 *    · Bloque 3 — Load (SimBrief): passengers, cargo, fuel.
 *    · Bloque 4 — Aircraft: type, registration (placeholder),
 *      airline (placeholder).
 *
 *  Cada bloque es una card independiente con borde redondeado, shadow
 *  sutil y padding generoso. AnimatePresence + motion.div para
 *  microanimaciones al cambiar de vuelo seleccionado. */
// (v4.11.0) ISO-2 → nombre de país legible con la API nativa del
// navegador (sin dataset extra). Usa el locale actual (es/en).
let REGION_DN: Intl.DisplayNames | null | undefined;
function countryName(iso?: string | null): string | null {
  if (!iso) return null;
  try {
    if (REGION_DN === undefined) {
      REGION_DN = new Intl.DisplayNames(undefined, { type: "region" });
    }
    return REGION_DN?.of(iso.toUpperCase()) ?? iso;
  } catch {
    return iso;
  }
}

/** "Ciudad, País" a partir del AirportBrief (omite lo que falte). */
function airportSubtitle(a?: AirportBrief): string | null {
  if (!a) return null;
  const parts: string[] = [];
  if (a.municipality) parts.push(a.municipality);
  const country = countryName(a.isoCountry);
  if (country) parts.push(country);
  return parts.length ? parts.join(", ") : null;
}

/** (v4.11.0) ICAO en negrita + nombre de aeropuerto y "ciudad, país"
 *  debajo. Usado en las filas Origin/Destination del bloque ROUTE. */
function AirportCell({
  icao,
  airport,
  fallbackName,
}: {
  icao: string;
  airport?: AirportBrief;
  fallbackName?: string | null;
}) {
  const name = airport?.name ?? fallbackName ?? null;
  const sub = airportSubtitle(airport);
  return (
    <span className="flex flex-col items-end text-right">
      <span className="font-mono font-bold text-slate-100">{icao}</span>
      {name && (
        <span className="font-sans text-[11px] font-normal leading-tight text-slate-400">
          {name}
        </span>
      )}
      {sub && (
        <span className="font-sans text-[10px] font-normal leading-tight text-slate-500">
          {sub}
        </span>
      )}
    </span>
  );
}

function SelectedFlightPanel({
  entry,
  collapsed,
  onToggleCollapse,
}: {
  entry: FlightLogEntry;
  collapsed: boolean;
  onToggleCollapse: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const reload = useFlightLogStore((s) => s.reload);
  // (v3.28.0 P7.11) Formateadores de unidades reactivos.
  const u = useUnits();

  // (v4.11.0) Resuelve nombre/ciudad/país de origen y destino por ICAO.
  const [airports, setAirports] = useState<Record<string, AirportBrief>>({});
  useEffect(() => {
    const icaos = [entry.originIcao, entry.destinationIcao].filter(
      (x): x is string => !!x,
    );
    if (icaos.length === 0) {
      setAirports({});
      return;
    }
    let alive = true;
    import("../lib/tauri")
      .then(({ api }) => api.lookupAirports(icaos))
      .then((rows) => {
        if (!alive) return;
        const map: Record<string, AirportBrief> = {};
        for (const r of rows) map[r.icao] = r;
        setAirports(map);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [entry.originIcao, entry.destinationIcao]);

  // (v3.3.0) Re-renderiza con animación al cambiar de vuelo.
  return (
    <AnimatePresence mode="wait">
      <motion.div
        key={entry.id}
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -8 }}
        transition={{ duration: 0.2 }}
        className="space-y-3"
      >
        {/* HEADER — Card prominente con ruta y aerolínea. */}
        <div className="rounded-2xl border border-amber-500/40 bg-gradient-to-br from-amber-500/[0.12] to-amber-500/[0.04] p-4 shadow-md shadow-amber-500/10 ring-1 ring-amber-500/20">
          <div className="mb-2 flex items-center justify-between gap-2">
            <div className="inline-flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-amber-300/90">
              <PlaneLanding className="h-3 w-3" />
              {t("fb.selected_flight")}
            </div>
            <div className="flex shrink-0 items-center gap-1">
              {/* (v3.6.1 fix I6 → v3.6.4 fix K5) Botón colapsar/expandir
                  con iconos lucide claros (Maximize2/Minimize2). El
                  usuario reportó que el punto "▴/▾" se veía confuso. */}
              <button
                onClick={onToggleCollapse}
                title={
                  collapsed
                    ? t("fb.detail.expand")
                    : t("fb.detail.collapse")
                }
                className="inline-flex items-center gap-1 rounded-full border border-amber-500/40 bg-amber-500/10 px-2.5 py-1 text-[10px] font-medium text-amber-200 transition-colors hover:bg-amber-500/20"
              >
                {collapsed ? (
                  <Maximize2 className="h-2.5 w-2.5" />
                ) : (
                  <Minimize2 className="h-2.5 w-2.5" />
                )}
                {collapsed ? t("fb.detail.expand_label") : t("fb.detail.collapse_label")}
              </button>
              <button
                onClick={() => setEditing(true)}
                title={t("fb.action.edit")}
                className="inline-flex items-center gap-1 rounded-full border border-amber-500/40 bg-amber-500/10 px-2.5 py-1 text-[10px] font-medium text-amber-200 transition-colors hover:bg-amber-500/20"
              >
                <Pencil className="h-2.5 w-2.5" />
                {t("fb.action.edit")}
              </button>
            </div>
          </div>
          {editing && (
            <EditFlightModal
              entry={entry}
              onClose={() => setEditing(false)}
              onSaved={() => {
                setEditing(false);
                void reload();
              }}
            />
          )}

          {/* (v4.8.0) LOGO + NOMBRE de aerolínea — identidad institucional
              del vuelo, justo encima de la ruta. Fallback elegante al
              código si el CDN no tiene el logo. */}
          {(entry.airlineIcao || entry.aircraftAirline) && (
            <div className="mb-1.5 flex items-center gap-2">
              <AirlineLogo
                icao={entry.airlineIcao}
                name={entry.aircraftAirline}
                size={28}
              />
              {entry.aircraftAirline && (
                <span className="text-xs font-semibold uppercase tracking-wide text-amber-200/90">
                  {entry.aircraftAirline}
                </span>
              )}
            </div>
          )}
          {/* (v3.4.0) ROUTE BIG — text-4xl font-black, mono,
              tracking-tight. Es el elemento más prominente del panel,
              lo PRIMERO que el usuario ve al abrir un vuelo. */}
          <div className="flex items-baseline gap-3 font-mono">
            <span className="text-4xl font-black tracking-tight text-amber-50 drop-shadow">
              {entry.originIcao ?? "?"}
            </span>
            <span className="text-2xl text-amber-300">→</span>
            <span className="text-4xl font-black tracking-tight text-amber-50 drop-shadow">
              {entry.destinationIcao ?? "?"}
            </span>
          </div>
          {/* AIRCRAFT + DATE — medium peso, secundario. */}
          <div className="mt-1.5 flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
            {(entry.aircraftAtcType || entry.aircraftTitle) && (
              <span className="text-sm font-medium text-amber-100">
                {cleanAircraftLabel(entry.aircraftAtcType ?? entry.aircraftTitle)}
              </span>
            )}
            <span className="text-[11px] tracking-wide text-amber-300/70">
              · {formatDate(entry.startedAt)}
            </span>
          </div>
        </div>

        {/* (v3.6.1 fix I6) Cuando el panel está colapsado, sólo se ve
            la card del header — el resto del mapa queda libre para que
            el usuario examine la trayectoria. */}
        {collapsed ? null : (
        <>
        {/* BLOQUE 1 — Route */}
        <DetailBlock title={t("fb.block.route")} icon="route">
          <BlockRow label={t("fb.route.origin")} value={
            <AirportCell
              icao={entry.originIcao ?? "?"}
              airport={
                entry.originIcao ? airports[entry.originIcao] : undefined
              }
              fallbackName={entry.originName}
            />
          } />
          <BlockRow label={t("fb.route.destination")} value={
            <AirportCell
              icao={entry.destinationIcao ?? "?"}
              airport={
                entry.destinationIcao
                  ? airports[entry.destinationIcao]
                  : undefined
              }
              fallbackName={entry.destinationName}
            />
          } />
          <BlockRow label={t("fb.route.distance")} value={u.fmt.distance(entry.distanceNm)} />
          {/* (v3.4.0) Gates SIEMPRE visibles aún sin datos —
              el usuario pidió que el panel de detalle muestre los gates
              de salida y llegada obligatoriamente. Cuando el watcher no
              los capturó renderizamos "—" en lugar de ocultar la fila.
              Eso preserva la simetría de bloques y deja claro qué
              campos NO se capturaron. */}
          <BlockRow
            label={t("fb.route.dep_gate")}
            value={
              entry.departureGate ? (
                <span className="font-mono text-sm font-semibold text-emerald-100">
                  {shortGate(entry.departureGate)}
                </span>
              ) : (
                <span className="text-slate-600">—</span>
              )
            }
          />
          <BlockRow
            label={t("fb.route.arr_gate")}
            value={
              entry.arrivalGate ? (
                <span className="font-mono text-sm font-semibold text-emerald-100">
                  {shortGate(entry.arrivalGate)}
                </span>
              ) : (
                <span className="text-slate-600">—</span>
              )
            }
          />
        </DetailBlock>

        {/* BLOQUE 2 — Times. (v3.5.0) Rediseñado para mostrar la
            timeline visual entre OUT (block-out) e IN (block-in) con
            una barra horizontal que representa la duración del vuelo.
            Los times aparecen anclados a los extremos de la barra. */}
        <DetailBlock title={t("fb.block.times")} icon="times">
          <FlightTimeline
            outIso={entry.startedAt}
            inIso={entry.endedAt}
            outLabel={t("fb.times.out")}
            inLabel={t("fb.times.in")}
          />
          <BlockRow
            label={t("fb.times.block")}
            value={
              <span className="font-mono font-semibold text-slate-100">
                {entry.flightTimeS !== null
                  ? formatHM(entry.flightTimeS)
                  : "—"}
              </span>
            }
          />
          <BlockRow
            label={t("fb.times.paused")}
            value={
              entry.pausedSeconds > 0 ? (
                <span className="font-mono text-[11px] text-slate-400">
                  {formatHM(entry.pausedSeconds)}
                </span>
              ) : (
                <span className="text-slate-600">—</span>
              )
            }
          />
        </DetailBlock>

        {/* BLOQUE 3 — Load (SimBrief) */}
        <DetailBlock title={t("fb.block.load")} icon="load">
          <BlockRow
            label={t("fb.load.passengers")}
            value={
              entry.passengers != null
                ? entry.passengers.toLocaleString("en-US")
                : "—"
            }
          />
          <BlockRow
            label={t("fb.load.cargo")}
            value={u.fmt.weight(entry.cargoKg)}
          />
          <BlockRow
            label={t("fb.load.fuel_used")}
            value={u.fmt.weight(entry.fuelUsedKg)}
          />
        </DetailBlock>

        {/* BLOQUE 4 — Aircraft. (v3.5.0 → v3.5.1) Layout vertical:
            foto grande del avión arriba (full width del block, aspect
            3:2 que es el ratio nativo de las fotos de planespotters),
            campos debajo. Cuando no hay matrícula válida la silueta
            SVG ocupa el mismo espacio. */}
        <DetailBlock title={t("fb.block.aircraft")} icon="aircraft">
          <div className="space-y-3">
            <AircraftThumb
              type={entry.aircraftAtcType}
              title={entry.aircraftTitle}
              airline={
                entry.aircraftAirline ??
                icaoToName(entry.airlineIcao) ??
                entry.airlineIcao
              }
              registration={entry.aircraftRegistration}
            />
            <div className="space-y-1.5">
              <BlockRow
                label={t("fb.aircraft.type")}
                value={entry.aircraftAtcType ?? "—"}
              />
              <BlockRow
                label={t("fb.aircraft.model")}
                value={entry.aircraftModel ?? "—"}
              />
              <BlockRow
                label={t("fb.aircraft.airline")}
                value={
                  entry.aircraftAirline ??
                  icaoToName(entry.airlineIcao) ??
                  entry.airlineIcao ??
                  "—"
                }
              />
              <BlockRow
                label={t("fb.aircraft.registration")}
                value={entry.aircraftRegistration ?? "—"}
              />
              <BlockRow
                label={t("fb.aircraft.title")}
                value={entry.aircraftTitle ?? "—"}
              />
            </div>
          </div>
        </DetailBlock>

        {/* (v3.6.7 fix N1) Para vuelos IMPORTADOS (source != simconnect)
            mostramos un badge "Imported flight" en vez de las peak
            metrics. El sampling de VAS-ACARS no permite calcular FPM
            con la precisión que reportan los VA platforms; preferible
            ser honesto y NO mostrar datos imprecisos.
            Para vuelos volados en vivo (SimConnect), las peak metrics
            sí son precisas (capturadas en tiempo real con simvars). */}
        {entry.source !== "simconnect" ? (
          <div className="flex items-center gap-2 rounded-xl border border-sky-500/30 bg-sky-500/5 px-4 py-2.5 text-[11px]">
            <span className="inline-flex items-center gap-1.5 font-medium text-sky-200">
              <span className="inline-block h-1.5 w-1.5 rounded-full bg-sky-400" />
              {t("fb.imported_flight.badge")}
            </span>
            <span className="text-slate-500">
              {t("fb.imported_flight.detail")}
            </span>
          </div>
        ) : (
          (entry.maxAltitudeFt !== null || entry.maxGroundSpeedKt !== null) && (
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-2.5 text-[11px]">
              <span className="text-slate-500">{t("fb.block.peak")}:</span>
              {entry.maxAltitudeFt !== null && (
                <span className="font-mono text-slate-200">
                  <span className="text-slate-400">ALT</span>{" "}
                  {u.fmt.altitude(entry.maxAltitudeFt)}
                </span>
              )}
              {entry.maxGroundSpeedKt !== null && (
                <span className="font-mono text-slate-200">
                  <span className="text-slate-400">GS</span>{" "}
                  {u.fmt.speed(entry.maxGroundSpeedKt)}
                </span>
              )}
            </div>
          )
        )}
        {/* Touchdown rate sólo para vuelos en vivo (SimConnect) — ahí
            el valor viene del simvar PLANE TOUCHDOWN NORMAL VELOCITY
            que es preciso. Para imports VAS-ACARS no se muestra.
            (v4.0.0 — P6.3 fix) Quitado el filtro `< 0`. El backend
            ahora garantiza que solo persiste valores negativos
            válidos en el watcher (fix en simconnect_watcher.rs). El
            usuario reportó que el FPM estaba en los logs pero no se
            pintaba — causa: valores positivos del simvar (bounce o
            modelado dudoso) pasaban el filtro `!== null` pero
            fallaban el `< 0` del frontend. */}
        {entry.source === "simconnect" && entry.landingFpm !== null && (() => {
          // (v4.0.0 — P6.3 iter 2) **Force-negate** el FPM en el
          // render. El simvar `PLANE TOUCHDOWN NORMAL VELOCITY` a
          // veces reporta valores positivos en aviones de terceros
          // o con bounces. La convención de aviación es siempre
          // mostrar el touchdown rate como NEGATIVO (descenso).
          // LandingToast hace lo mismo — siempre muestra valores
          // negativos en su UI aunque el simvar diga otra cosa.
          //
          // Esto también arregla los vuelos VIEJOS donde el backend
          // pre-v3.10.0 guardó valores positivos por el bug del
          // filtro `abs()`. Force-negate en el render sin tocar la DB.
          const fpm = -Math.abs(entry.landingFpm);
          // (v4.0.0 P7.6b) Escala de aviación real:
          //   > -100  → cyan (flotado / float landing)
          //   -100..-299 → verde (aterrizaje óptimo)
          //   -300..-399 → amarillo (duro / al límite)
          //   ≤ -400  → rojo (peligroso / estructural)
          const color =
            fpm > -100
              ? "text-cyan-300"
              : fpm > -300
                ? "text-emerald-300"
                : fpm > -400
                  ? "text-amber-300"
                  : "text-rose-400 font-semibold";
          return (
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-2.5 text-[11px]">
              <span className="text-slate-500">{t("fb.block.touchdown")}:</span>
              <span className={`font-mono ${color}`}>
                {u.fmt.vs(fpm)}
              </span>
              {entry.bounced && (
                <span
                  className="rounded-md border border-amber-400/40 bg-amber-500/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-300"
                  title={t("fb.block.bounce_tooltip")}
                >
                  ↑ {t("fb.block.bounce_detected")}
                </span>
              )}
            </div>
          );
        })()}

        {/* (v4.0.0 P7.4b) Temperatura máxima de frenos — solo si el
            puente LVar de MobiFlight la capturó (FBW A32NX, etc.).
            Ausente en la gran mayoría de vuelos. */}
        {entry.maxBrakeTempC != null && entry.maxBrakeTempC > 0 && (() => {
          const c = entry.maxBrakeTempC;
          // Escala orientativa de frenos de airliner:
          //   < 200°C  → verde (normal)
          //   200-349  → amarillo (caliente, normal tras frenado fuerte)
          //   ≥ 350°C  → rojo (muy caliente / posible overheat)
          const color =
            c < 200
              ? "text-emerald-300"
              : c < 350
                ? "text-amber-300"
                : "text-rose-400 font-semibold";
          return (
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-2.5 text-[11px]">
              <span className="text-slate-500">
                {t("fb.block.brake_temp")}:
              </span>
              <span className={`font-mono ${color}`}>{u.fmt.temp(c)}</span>
            </div>
          );
        })()}
        </>
        )}
      </motion.div>
    </AnimatePresence>
  );
}

/** (v3.3.0) Card de bloque para el detalle del vuelo. Borde
 *  redondeado, shadow sutil, padding generoso. Header con icono +
 *  título en uppercase tracking. Children son las rows. */
function DetailBlock({
  title,
  icon,
  children,
}: {
  title: string;
  icon: "route" | "times" | "load" | "aircraft";
  children: React.ReactNode;
}) {
  const Icon = ({}: Record<string, never>) => {
    switch (icon) {
      case "route":
        return <MapPin className="h-3.5 w-3.5 text-sky-300" />;
      case "times":
        return <Clock className="h-3.5 w-3.5 text-emerald-300" />;
      case "load":
        return <Package className="h-3.5 w-3.5 text-violet-300" />;
      case "aircraft":
        return <Plane className="h-3.5 w-3.5 text-amber-300" />;
    }
  };
  return (
    <div className="rounded-2xl border border-slate-800 bg-slate-900/40 p-4 shadow-sm transition-shadow hover:shadow-md">
      <div className="mb-2.5 inline-flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-slate-400">
        <Icon />
        {title}
      </div>
      <div className="space-y-1.5">{children}</div>
    </div>
  );
}

/** (v3.3.0) Row de un bloque — label izquierda en slate-500, value
 *  derecha en slate-100. Padding vertical generoso para legibilidad. */
function BlockRow({
  label,
  value,
}: {
  label: string;
  value: React.ReactNode;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-[12px]">
      <span className="shrink-0 text-slate-500">{label}</span>
      <span className="text-right text-slate-100">{value}</span>
    </div>
  );
}

/**
 * Format HH:MM in the user's LOCAL timezone from an ISO timestamp.
 * (v3.5.0) — previously rendered UTC with a "Z" suffix; the user
 * preferred seeing block-out/landing times in their wall clock.
 */
function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    const hh = d.getHours().toString().padStart(2, "0");
    const mm = d.getMinutes().toString().padStart(2, "0");
    return `${hh}:${mm}`;
  } catch {
    return "—";
  }
}

// (v4.18.1) `ActiveFlightCard` ("FLYING NOW") eliminado — el
// LiveFlightPanel del RoutesMapView muestra el vuelo activo con más
// detalle; el overlay queda para el detalle del vuelo seleccionado.

// (v3.3.0) `Metric` legacy component eliminado — DetailBlock +
// BlockRow lo reemplazan completamente.

// (v4.19.0) `PreflightCard` ("ON GROUND · PRE-FLIGHT") eliminado —
// feedback usuario: el overlay del mapa solo muestra el detalle del
// vuelo seleccionado; el estado en vivo lo cubre el LiveFlightPanel.

/** (v3.5.0 F4) Widget compacto del resumen — antes ocupaba todo el
 *  mapa con 7 StatCards en grid 2-col. Ahora es una pila vertical
 *  densa de filas: ícono + label inline + valor a la derecha. Ocupa
 *  ~3× menos espacio.
 *
 *  El FPM promedio usa **color coding** según calidad del aterrizaje
 *  (literatura aviation: verde mariposa, slate firme, rojo duro) —
 *  el mismo coding que el detail panel de cada vuelo, así el usuario
 *  ve el promedio coloreado en el resumen sin tener que abrir vuelos
 *  individuales.
 */
function StatsWidget({
  stats,
}: {
  stats: {
    count: number;
    totalSec: number;
    totalDistance: number;
    totalPassengers: number;
    totalCargoKg: number;
    totalFuelKg: number;
    passengersFlightCount: number;
    cargoFlightCount: number;
    fuelFlightCount: number;
    avgLandingFpm: number | null;
    fpmFlightCount: number;
  };
}) {
  const u = useUnits();
  const fpmColor =
    stats.avgLandingFpm == null
      ? "text-slate-200"
      : stats.avgLandingFpm > -200
        ? "text-emerald-300"
        : stats.avgLandingFpm > -500
          ? "text-slate-200"
          : "text-rose-300";

  return (
    <div className="space-y-1.5">
      <div className="mb-1 inline-flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.15em] text-emerald-300">
        <Plane className="h-3 w-3" />
        {t("fb.stats.overview")}
      </div>
      <div className="divide-y divide-slate-800/80 overflow-hidden rounded-lg border border-slate-800 bg-slate-900/40">
        <CompactRow
          icon={<Plane className="h-3 w-3 text-emerald-300" />}
          label={t("fb.stat.flights")}
          value={stats.count.toString()}
        />
        <CompactRow
          icon={<Clock className="h-3 w-3 text-sky-300" />}
          label={t("fb.stat.total_time")}
          value={formatHM(stats.totalSec)}
        />
        <CompactRow
          icon={<Ruler className="h-3 w-3 text-violet-300" />}
          label={t("fb.stat.distance")}
          value={u.fmt.distance(stats.totalDistance)}
        />
        <CompactRow
          icon={<Users className="h-3 w-3 text-emerald-300" />}
          label={t("fb.stat.passengers")}
          value={
            stats.passengersFlightCount > 0
              ? stats.totalPassengers.toLocaleString("en-US")
              : "—"
          }
        />
        <CompactRow
          icon={<Package className="h-3 w-3 text-violet-300" />}
          label={t("fb.stat.cargo")}
          value={
            stats.cargoFlightCount > 0
              ? `${(stats.totalCargoKg / 1000).toFixed(1)} t`
              : "—"
          }
        />
        <CompactRow
          icon={<Droplet className="h-3 w-3 text-sky-300" />}
          label={t("fb.stat.total_fuel")}
          value={
            stats.fuelFlightCount > 0
              ? `${(stats.totalFuelKg / 1000).toFixed(1)} t`
              : "—"
          }
        />
        <CompactRow
          icon={<Plane className="h-3 w-3 text-amber-300" />}
          label={t("fb.stat.avg_landing_fpm")}
          value={u.fmt.vs(stats.avgLandingFpm)}
          valueClassName={fpmColor}
        />
      </div>
    </div>
  );
}

function CompactRow({
  icon,
  label,
  value,
  valueClassName,
}: {
  icon: React.ReactNode;
  label: string;
  value: string;
  valueClassName?: string;
}) {
  return (
    <div className="flex items-center justify-between px-2.5 py-1.5">
      <div className="flex min-w-0 items-center gap-1.5 text-[10px] uppercase tracking-wide text-slate-400">
        {icon}
        <span className="truncate">{label}</span>
      </div>
      <span
        className={`shrink-0 font-mono text-xs font-semibold ${
          valueClassName ?? "text-slate-100"
        }`}
      >
        {value}
      </span>
    </div>
  );
}

/** (v3.5.0) Timeline visual entre OUT (block-out) e IN (block-in).
 *  Renderiza una barra horizontal con los times anclados a sus
 *  extremos: pista de fondo + tracker activo desde el inicio hasta
 *  el fin (o hasta "ahora" si el vuelo sigue abierto). Style: minimal
 *  + monoespaciado para los números.
 */
function FlightTimeline({
  outIso,
  inIso,
  outLabel,
  inLabel,
}: {
  outIso: string;
  inIso: string | null;
  outLabel: string;
  inLabel: string;
}) {
  const outTime = formatTime(outIso);
  const inTime = inIso ? formatTime(inIso) : "—";
  // Progress sólo significativo cuando el vuelo está cerrado.
  // Para vuelos abiertos rendereamos el tracker al 100% como visual
  // hint de "está en curso" — pero también podríamos animarlo.
  const closed = inIso !== null;
  return (
    <div className="space-y-2 py-1">
      {/* Track con pista + barra activa + extremos circulares. */}
      <div className="relative mx-1 h-1.5 rounded-full bg-slate-800">
        <div
          className="absolute inset-y-0 left-0 rounded-full bg-emerald-400"
          style={{ width: closed ? "100%" : "55%" }}
        />
        <div className="absolute -left-1 top-1/2 h-3 w-3 -translate-y-1/2 rounded-full border-2 border-emerald-400 bg-slate-950" />
        <div
          className={`absolute -right-1 top-1/2 h-3 w-3 -translate-y-1/2 rounded-full border-2 bg-slate-950 ${
            closed ? "border-emerald-400" : "border-slate-600"
          }`}
        />
      </div>
      {/* Labels alineadas a los extremos. */}
      <div className="flex items-baseline justify-between text-[10px] uppercase tracking-wider text-slate-500">
        <div className="flex flex-col items-start">
          <span>{outLabel}</span>
          <span className="font-mono text-sm text-slate-100">{outTime}</span>
        </div>
        <div className="flex flex-col items-end">
          <span>{inLabel}</span>
          <span className="font-mono text-sm text-slate-100">{inTime}</span>
        </div>
      </div>
    </div>
  );
}

/** (v3.5.0) Cache PERSISTENTE de fotos de planespotters indexada por
 *  matrícula. Dos niveles:
 *    · Memoria (`memCache`): Map para hits inmediatos sin parse JSON.
 *    · localStorage (`STORAGE_KEY`): persiste entre sesiones para que
 *      al re-entrar al FlightBook las fotos aparezcan sin "cargando".
 *  TTL: 30 días — suficiente para evitar refetches frecuentes pero
 *  permitir renovación si la API de planespotters cambia URLs.
 *  Valor `null` = se intentó y no había foto / falló (no reintenta
 *  hasta que expire la entrada).
 */
const PLANESPOTTERS_STORAGE_KEY = "simfleet:planespotters-cache:v1";
const PLANESPOTTERS_TTL_MS = 30 * 24 * 60 * 60 * 1000; // 30 días

type PlanespottersEntry = { url: string | null; ts: number };

const memCache = new Map<
  string,
  PlanespottersEntry & { inflight?: Promise<string | null> }
>();

/** Lee toda la persisted store al cargar el módulo (una sola vez). */
function loadPersistedCache(): Record<string, PlanespottersEntry> {
  try {
    const raw = localStorage.getItem(PLANESPOTTERS_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, PlanespottersEntry>;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

/** Guarda toda la persisted store (debounce-less; el set es pequeño
 *  y las escrituras son infrecuentes — una por matrícula nueva). */
function persistCache(store: Record<string, PlanespottersEntry>): void {
  try {
    localStorage.setItem(PLANESPOTTERS_STORAGE_KEY, JSON.stringify(store));
  } catch {
    // localStorage lleno o disabled — ignoramos, mem cache sigue ok.
  }
}

// Hidratamos el mem cache desde localStorage al cargar el módulo.
// Esto significa que cuando un componente AircraftThumb monta por
// primera vez en la sesión, ya tiene los hits previos en memoria.
(() => {
  const persisted = loadPersistedCache();
  const now = Date.now();
  for (const [reg, entry] of Object.entries(persisted)) {
    if (now - entry.ts < PLANESPOTTERS_TTL_MS) {
      memCache.set(reg, entry);
    }
  }
})();

/** Resuelve la URL de la foto thumbnail desde la API pública de
 *  planespotters.net. Endpoint:
 *    GET https://api.planespotters.net/pub/photos/reg/{reg}
 *  Devuelve `{ photos: [{ thumbnail_large: { src } }, ...] }`. */
async function fetchPlanespottersPhoto(reg: string): Promise<string | null> {
  const cached = memCache.get(reg);
  if (cached) {
    // Si la entrada está fresca (dentro del TTL), úsala sin refetch.
    if (Date.now() - cached.ts < PLANESPOTTERS_TTL_MS) {
      if (cached.inflight) return cached.inflight;
      return cached.url;
    }
  }
  const inflight = (async () => {
    try {
      // (v6.2.3) Se pide desde Rust: planespotters ahora EXIGE un User-Agent
      // con contacto, que el `fetch()` del webview no puede fijar (header
      // prohibido) — por eso dejó de salir CUALQUIER foto. El backend manda el
      // UA correcto y devuelve sólo la URL; el <img> la carga normal.
      const { api } = await import("../lib/tauri");
      return await api.aircraftPhoto(reg);
    } catch {
      return null;
    }
  })();
  memCache.set(reg, { url: null, ts: Date.now(), inflight });
  const url = await inflight;
  const entry: PlanespottersEntry = { url, ts: Date.now() };
  memCache.set(reg, entry);
  // Persistimos toda la mem cache (sin promises) — el set es pequeño.
  const toStore: Record<string, PlanespottersEntry> = {};
  for (const [r, e] of memCache.entries()) {
    if (!e.inflight) toStore[r] = { url: e.url, ts: e.ts };
  }
  persistCache(toStore);
  return url;
}

/** Hook React que devuelve la URL de la foto cuando la registration
 *  existe y la API responde. Si la entrada está en cache (memoria o
 *  localStorage), devuelve el valor inmediatamente sin parpadeo. */
export function useAircraftPhoto(registration: string | null): string | null {
  const initial = (() => {
    if (!registration) return null;
    const reg = registration.trim().toUpperCase();
    if (!reg || !/^[A-Z0-9-]{3,10}$/.test(reg)) return null;
    const cached = memCache.get(reg);
    if (cached && Date.now() - cached.ts < PLANESPOTTERS_TTL_MS) {
      return cached.url;
    }
    return null;
  })();
  const [url, setUrl] = useState<string | null>(initial);
  useEffect(() => {
    if (!registration) {
      setUrl(null);
      return;
    }
    const reg = registration.trim().toUpperCase();
    if (!reg || !/^[A-Z0-9-]{3,10}$/.test(reg)) {
      setUrl(null);
      return;
    }
    // Cache hit — usa el valor sincrónicamente para evitar parpadeo
    // (silueta → foto). Si ya está fresco, no refetch.
    const cached = memCache.get(reg);
    if (cached && !cached.inflight && Date.now() - cached.ts < PLANESPOTTERS_TTL_MS) {
      setUrl(cached.url);
      return;
    }
    let cancelled = false;
    void fetchPlanespottersPhoto(reg).then((u) => {
      if (!cancelled) setUrl(u);
    });
    return () => {
      cancelled = true;
    };
  }, [registration]);
  return url;
}

/** (v3.5.0) Aircraft thumbnail. Si la matrícula trae una foto desde
 *  planespotters.net, la muestra; si no, cae a una silueta SVG
 *  derivada del ATC type/title con color determinístico por airline.
 *  La transición silueta → foto es instantánea (sin spinner) para
 *  no parpadear: la silueta se queda como placeholder permanente
 *  cuando no hay foto disponible. */
function AircraftThumb({
  type,
  title,
  airline,
  registration,
}: {
  type: string | null;
  title: string | null;
  airline: string | null;
  registration: string | null;
}) {
  const photoUrl = useAircraftPhoto(registration);
  const haystack = `${type ?? ""} ${title ?? ""}`.toLowerCase();
  // Color del avión deriva del airline para sensación de "livery".
  const liveryColor = colorFromString(airline ?? type ?? "default");
  // Familia: usamos las siluetas más comunes en MSFS.
  let family: "narrow" | "wide" | "regional" | "turboprop" = "narrow";
  if (/(74[478]|777|78[0-9]|a3[345]0|a380|md-?11)/i.test(haystack)) {
    family = "wide";
  } else if (/(crj|atr-?7|erj|e[1-2]9[05]|e1[57][05])/i.test(haystack)) {
    family = "regional";
  } else if (/(atr|dash|q4|tbm|c208|king ?air|kodiak|pc-?12)/i.test(haystack)) {
    family = "turboprop";
  }
  return (
    <div
      className="relative flex aspect-[3/2] w-full items-center justify-center overflow-hidden rounded-xl border border-slate-800 bg-gradient-to-br from-slate-900 to-slate-950 ring-1 ring-slate-800"
      title={
        registration && photoUrl
          ? `${registration} · planespotters.net`
          : title ?? type ?? ""
      }
    >
      {photoUrl ? (
        <img
          src={photoUrl}
          alt={`${registration ?? "aircraft"} livery`}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          className="h-full w-full object-cover"
          onError={(ev) => {
            // Si la imagen falla (404 / hot-link), la ocultamos y la
            // silueta queda visible debajo.
            (ev.currentTarget as HTMLImageElement).style.display = "none";
          }}
        />
      ) : (
        <AircraftSilhouette family={family} fill={liveryColor} size="large" />
      )}
      {/* Sutil overlay para textura — sólo cuando NO hay foto, para
          no oscurecer la foto real. */}
      {!photoUrl && (
        <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-slate-950/60 to-transparent" />
      )}
      {/* Crédito sutil de planespotters cuando se muestra una foto real. */}
      {photoUrl && (
        <span className="pointer-events-none absolute bottom-1 right-1.5 rounded bg-slate-950/60 px-1 py-0.5 text-[8px] font-medium uppercase tracking-wider text-slate-200/80 backdrop-blur-sm">
          planespotters.net
        </span>
      )}
      {airline && !photoUrl && (
        <span className="pointer-events-none absolute bottom-2 left-2 right-2 truncate text-center text-[11px] font-semibold uppercase tracking-wider text-white/90">
          {airline}
        </span>
      )}
    </div>
  );
}

/** SVG silhouette mínimo por familia. Stroke + fill controlados por
 *  la prop `fill` (color derivado del airline en `AircraftThumb`).
 *  `size` controla el tamaño del SVG dentro del contenedor — "large"
 *  para el banner full-width, default para usos compactos. */
function AircraftSilhouette({
  family,
  fill,
  size = "small",
}: {
  family: "narrow" | "wide" | "regional" | "turboprop";
  fill: string;
  size?: "small" | "large";
}) {
  const common =
    size === "large"
      ? "h-24 w-24 drop-shadow-lg"
      : "h-12 w-12 drop-shadow";
  if (family === "wide") {
    // Wide-body: fuselaje grueso + 4 winglets sutiles.
    return (
      <svg viewBox="0 0 64 64" className={common} aria-hidden>
        <g fill={fill} stroke={fill} strokeWidth="0.5">
          <path d="M32 6 C36 12 36 24 36 30 L58 38 L58 41 L36 36 L36 50 L42 54 L42 56 L32 53 L22 56 L22 54 L28 50 L28 36 L6 41 L6 38 L28 30 C28 24 28 12 32 6 Z" />
        </g>
      </svg>
    );
  }
  if (family === "regional") {
    // Regional jet: cuerpo más esbelto, T-tail sugerido.
    return (
      <svg viewBox="0 0 64 64" className={common} aria-hidden>
        <g fill={fill} stroke={fill} strokeWidth="0.5">
          <path d="M32 8 C35 14 35 26 35 32 L54 39 L54 41 L35 37 L35 49 L40 53 L40 55 L32 52 L24 55 L24 53 L29 49 L29 37 L10 41 L10 39 L29 32 C29 26 29 14 32 8 Z" />
        </g>
      </svg>
    );
  }
  if (family === "turboprop") {
    // Turboprop: alas más altas + nariz de hélice (círculo).
    return (
      <svg viewBox="0 0 64 64" className={common} aria-hidden>
        <g fill={fill} stroke={fill} strokeWidth="0.5">
          <circle cx="32" cy="8" r="2.5" />
          <path d="M32 11 C35 17 35 26 35 32 L52 38 L52 40 L35 36 L35 48 L39 52 L39 54 L32 51 L25 54 L25 52 L29 48 L29 36 L12 40 L12 38 L29 32 C29 26 29 17 32 11 Z" />
        </g>
      </svg>
    );
  }
  // Narrow-body default (737/A320 family).
  return (
    <svg viewBox="0 0 64 64" className={common} aria-hidden>
      <g fill={fill} stroke={fill} strokeWidth="0.5">
        <path d="M32 7 C35 13 35 25 35 31 L56 38 L56 40 L35 36 L35 50 L40 54 L40 56 L32 53 L24 56 L24 54 L29 50 L29 36 L8 40 L8 38 L29 31 C29 25 29 13 32 7 Z" />
      </g>
    </svg>
  );
}

/** Deriva un color HSL determinístico desde un string (nombre de
 *  aerolínea / type). Idéntica entrada → idéntico color, así cada
 *  airline mantiene su livery consistente entre vuelos. */
function colorFromString(s: string): string {
  let hash = 0;
  for (let i = 0; i < s.length; i++) {
    hash = (hash * 31 + s.charCodeAt(i)) & 0xffffffff;
  }
  const hue = Math.abs(hash) % 360;
  return `hsl(${hue} 65% 60%)`;
}

/** (v3.5.0) Tabs deshabilitadas que viven sobre la card de detalle.
 *  Secciones: Checklist (procedures), Weather (METAR / TAF + Windy),
 *  NOTAMs (SimBrief OFP).
 *  Hoy renderizan opacas + cursor not-allowed; cada una muestra un dot
 *  de color para identificar visualmente la categoría. Cuando se
 *  implementen, se quitará el `disabled` y se conectará el handler.
 *
 *  (v3.6.0 Phase H — Epic E) **Checklist** YA ESTÁ habilitada cuando
 *  hay un vuelo seleccionado. El click abre el ChecklistWidget como
 *  glass overlay sobre el mapa.
 */
function DetailActionsBar({
  selectedFlightId,
  selectedFlightSource,
  onToggleChecklist,
  checklistOpen,
  onToggleVs,
  vsOpen,
  vsHasSnapshot,
}: {
  selectedFlightId: number | null;
  selectedFlightSource: string | null;
  onToggleChecklist: () => void;
  checklistOpen: boolean;
  onToggleVs: () => void;
  vsOpen: boolean;
  vsHasSnapshot: boolean;
}) {
  const checklistDisabled = selectedFlightId == null;
  // (v4.0.0 — P3 + P3.2) Tabs gated por source del vuelo:
  //   · **Checklist** — solo vuelos hechos en SimFleet (`simconnect`).
  //     Para VAS imports el rubric depende de simvars que no existen
  //     en el .bin (lights, pitch, etc.) → el score sería engañoso.
  //   · **Weather** — solo vuelos `simconnect`. Imports no tienen
  //     historical wind/clouds y las APIs gratuitas no permiten
  //     query histórica.
  //   · **NOTAMs** sigue visible para cualquier vuelo seleccionado
  //     (el modal resuelve si el OFP de SimBrief matchea).
  const isSimflownFlight = selectedFlightSource === "simconnect";
  const allTabs = [
    {
      key: "checklist" as const,
      icon: <ClipboardCheck className="h-3.5 w-3.5" />,
      label: t("fb.tabs.checklist"),
      dot: "bg-emerald-400",
      enabled: !checklistDisabled,
      onClick: onToggleChecklist,
      active: checklistOpen,
    },
    {
      // (v6.2.11) Crew VS — HISTÓRICO por vuelo. Habilitado SÓLO si ese vuelo
      // tiene un duelo guardado (Héctor también lo voló). Muestra el duelo de
      // ESE vuelo, no el en vivo.
      key: "vs" as const,
      icon: <Swords className="h-3.5 w-3.5" />,
      label: t("vs.tab"),
      dot: vsHasSnapshot ? "bg-fuchsia-400" : "bg-slate-500",
      enabled: vsHasSnapshot,
      onClick: onToggleVs,
      active: vsOpen,
    },
    // (v4.23.0) Tab de NOTAMs eliminada — pedido del usuario ("esto no
    // me interesa ya"). NotamsModal borrado junto con sus i18n keys.
  ];
  const tabs = allTabs.filter((t) => {
    if (t.key === "checklist" && !isSimflownFlight) return false;
    if (t.key === "vs" && !isSimflownFlight) return false;
    return true;
  });
  // Grid responsive: el ancho de los tabs se ajusta según cuántos
  // sobreviven el filtrado. Para VAS imports queda 1 (NOTAMs); para
  // vuelos SimFleet quedan 3 (Checklist + Weather + NOTAMs).
  const gridCols =
    tabs.length >= 4
      ? "grid-cols-4"
      : tabs.length === 3
        ? "grid-cols-3"
        : tabs.length === 2
          ? "grid-cols-2"
          : "grid-cols-1";
  return (
    <div className={`grid ${gridCols} gap-1.5`}>
      {tabs.map((tab) => (
        <button
          key={tab.label}
          disabled={!tab.enabled}
          onClick={tab.onClick}
          title={
            tab.enabled
              ? tab.active
                ? t("fb.checklist.close.tooltip")
                : t("fb.checklist.open.tooltip")
              : tab.key === "vs"
                ? t("vs.waiting")
                : checklistDisabled && tab.key === "checklist"
                  ? t("fb.checklist.disabled.tooltip")
                  : t("fb.tabs.coming_soon", { feature: tab.label })
          }
          className={`relative flex flex-col items-center gap-1 rounded-xl border px-2 py-2 text-[10px] font-medium ring-1 backdrop-blur-xl transition-all ${
            tab.active
              ? tab.key === "vs"
                ? "border-fuchsia-500/60 bg-fuchsia-500/15 text-fuchsia-100 ring-fuchsia-500/30"
                : "border-emerald-500/60 bg-emerald-500/15 text-emerald-100 ring-emerald-500/30"
              : tab.enabled
                ? "border-slate-700/60 bg-slate-950/65 text-slate-200 ring-slate-800/60 hover:border-slate-600 hover:bg-slate-900/75"
                : "border-slate-700/60 bg-slate-950/65 text-slate-300 opacity-60 ring-slate-800/60 disabled:cursor-not-allowed"
          }`}
        >
          <span
            className={`absolute right-1.5 top-1.5 inline-block h-1.5 w-1.5 rounded-full ${tab.dot}`}
            aria-hidden
          />
          <span className="text-slate-200">{tab.icon}</span>
          <span className="leading-none">{tab.label}</span>
        </button>
      ))}
    </div>
  );
}

/** (v3.5.0) Sidebar row — single-line compact representation of a
 *  flight, designed to fit ~12+ rows in a 300px column.
 *  Layout:
 *    ICAO → ICAO          duration
 *    aircraft · date
 *  Selected state: emerald accent + left-border ribbon.
 *  Hover trash icon on the right.
 */
function CompactFlightRow({
  entry,
  selected,
  onSelect,
  onDelete,
}: {
  entry: FlightLogEntry;
  selected: boolean;
  onSelect: () => void;
  onDelete: () => void;
}) {
  const duration =
    entry.flightTimeS !== null ? formatHM(entry.flightTimeS) : "—";
  const aircraft = cleanAircraftLabel(entry.aircraftAtcType ?? entry.aircraftTitle);
  // (v4.0.0 — P3.3 + P3.4) Vuelo importado (VAS-ACARS u otro): se
  // muestra un badge "Imported" y se **oculta** el grade del score.
  // El rubric depende de simvars que el .bin no expone (lights,
  // pitch, gear, flaps, etc.) — la mayoría de reglas saldrían
  // "skipped" y el grade resultante es engañoso. El usuario decidió
  // dejar esos vuelos sin score visible en el sidebar.
  const isImported = entry.source !== "simconnect";
  const showScore = !!entry.scoreGrade && !isImported;
  return (
    <li
      onClick={onSelect}
      title={
        selected ? t("fb.action.deselect") : t("fb.card.select_tooltip")
      }
      className={`group relative cursor-pointer overflow-hidden rounded-lg px-2.5 py-2 transition-colors ${
        selected
          ? "bg-amber-500/15 ring-1 ring-amber-500/40"
          : "hover:bg-slate-800/60"
      }`}
    >
      {selected && (
        <span className="absolute left-0 top-0 h-full w-0.5 bg-amber-400" />
      )}
      <div className="flex items-center justify-between gap-2 font-mono">
        <span className="flex min-w-0 items-center gap-1.5">
          {/* (v4.8.0) Logo de aerolínea mini — identidad visual rápida
              en el feed. Fallback a chip de código si no hay logo. */}
          <AirlineLogo
            icao={entry.airlineIcao}
            name={entry.aircraftAirline}
            size={16}
          />
          <span
            className={`truncate text-sm font-bold tracking-tight ${
              selected ? "text-amber-50" : "text-slate-100"
            }`}
          >
            <span>{entry.originIcao ?? "?"}</span>
            <span
              className={`mx-1 ${
                selected ? "text-amber-300" : "text-emerald-400"
              }`}
            >
              →
            </span>
            <span>{entry.destinationIcao ?? "?"}</span>
          </span>
        </span>
        <span className="shrink-0 font-sans text-[10px] tabular-nums text-slate-400">
          {duration}
        </span>
      </div>
      {/* (v3.6.0 Phase H — Epic C → v3.6.1 fix I1) **Callsign** preferido
          sobre flight_number. El usuario reportó: "DAL1803 / JBU8774 /
          AAL9833" (3-letter ICAO + número) es el formato que usa para
          identificar vuelos, no el IATA "DL3880". Fallback a
          flight_number si no hay callsign capturado. */}
      {(entry.callsign || entry.flightNumber || showScore || isImported) && (
        <div className="mt-0.5 flex items-center justify-between gap-2 text-[10px]">
          <span className="truncate font-mono font-semibold text-sky-300">
            {entry.callsign ?? entry.flightNumber ?? ""}
          </span>
          {isImported ? (
            <span
              title={t("fb.imported_flight.detail")}
              className="shrink-0 rounded bg-sky-500/15 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-sky-300 ring-1 ring-sky-500/30"
            >
              {t("fb.imported_flight.badge")}
            </span>
          ) : showScore ? (
            <span
              title={
                entry.scoreTotal !== null && entry.scoreMax !== null
                  ? `${entry.scoreTotal}/${entry.scoreMax}`
                  : undefined
              }
              className={`shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] font-bold ${gradeBadgeClass(
                entry.scoreGrade!,
              )}`}
            >
              {entry.scoreGrade}
            </span>
          ) : null}
        </div>
      )}
      <div className="mt-0.5 flex items-center justify-between gap-2 text-[10px] text-slate-500">
        <span className="truncate">
          {aircraft && <span className="text-slate-400">{aircraft}</span>}
          {aircraft && <span className="mx-1 text-slate-700">·</span>}
          {formatDate(entry.startedAt)}
        </span>
        <button
          onClick={(ev) => {
            ev.stopPropagation();
            onDelete();
          }}
          title={t("fb.delete.confirm")}
          className="shrink-0 rounded p-0.5 text-slate-700 opacity-0 transition-colors hover:bg-rose-500/15 hover:text-rose-300 group-hover:opacity-100"
        >
          <Trash2 className="h-2.5 w-2.5" />
        </button>
      </div>
    </li>
  );
}

/** Acorta el formato del gate fallback. Devuelve el string original
 *  cuando ya viene en un formato corto ("Stand · ...") o cuando no
 *  matchea ningún patrón conocido. */
function shortGate(gate: string): string {
  // Nuevo formato: "Stand · 320° 280m de EBBR" — ya es corto.
  if (gate.startsWith("Stand")) return gate;
  // Formato legacy: "Position: 50.9012, 4.4844"
  const m = gate.match(/^Position:\s*(-?\d+(?:\.\d+)?),\s*(-?\d+(?:\.\d+)?)/);
  if (!m) return gate;
  const lat = Number(m[1]).toFixed(2);
  const lon = Number(m[2]).toFixed(2);
  return `${lat},${lon}`;
}

/** (v3.6.0 Phase H — Epic E) Clase Tailwind del badge de grade en
 *  la sidebar. Grade A verde, B sky, C amber, D orange, F rose. */
function gradeBadgeClass(grade: string): string {
  switch (grade) {
    case "A":
      return "bg-emerald-500/20 text-emerald-200 ring-1 ring-emerald-500/40";
    case "B":
      return "bg-sky-500/20 text-sky-200 ring-1 ring-sky-500/40";
    case "C":
      return "bg-amber-500/20 text-amber-200 ring-1 ring-amber-500/40";
    case "D":
      return "bg-orange-500/20 text-orange-200 ring-1 ring-orange-500/40";
    default:
      return "bg-rose-500/20 text-rose-200 ring-1 ring-rose-500/40";
  }
}

function formatHM(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0m";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h === 0) return `${m}m`;
  return `${h}h ${m.toString().padStart(2, "0")}m`;
}

function formatDate(iso: string): string {
  try {
    const d = new Date(iso);
    // (v3.4.0) Locale resuelto del módulo i18n — refleja la
    // preferencia persistida del usuario (ES/EN). Antes hardcodeaba
    // "es-ES" y los abreviados de mes salían en español aún si el
    // usuario tenía la app en EN.
    const intl = getActiveLocale() === "es" ? "es-ES" : "en-US";
    return d.toLocaleString(intl, {
      day: "2-digit",
      month: "short",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso.slice(0, 10);
  }
}

/** (v3.6.0 Phase H — Epic D) Tira horizontal de chips de aerolíneas
 *  flotando sobre el mapa. Cada chip representa una airline con vuelos
 *  en el historial. Click toggle = activa/desactiva el filtro. Cuando
 *  hay un chip activo, el sidebar + el mapa + las KPIs se recalculan
 *  para esa aerolínea.
 *
 *  Layout: row scrollable horizontal, max-width contenido para no
 *  invadir el centro del mapa. Posición absolute con el suficiente
 *  z-index para no interferir con los controles del MapLibre.
 */
function AirlineTagFilter() {
  const airlines = useFlightLogStore((s) => s.airlines);
  const selected = useFlightLogStore((s) => s.selectedAirline);
  const setSelected = useFlightLogStore((s) => s.setSelectedAirline);
  // (v3.6.3 fix J4) Auto-ocultar tags con flight_count = 0 (puede
  // pasar si el listado se desactualiza).
  // (v6.2.36) MERGE de logos duplicados: el backend puede emitir varios
  // tags para la misma aerolínea (variantes de espacios/mayúsculas, o
  // icao vs aircraft_airline sucio como "DAL (N374DA)"). Los colapsamos
  // por clave canónica, SUMANDO vuelos, y nos quedamos con la variante de
  // más vuelos como representante (la que tiene icao gana el desempate).
  const visible = useMemo(() => {
    const byKey = new Map<string, AirlineTag>();
    for (const a of airlines) {
      if (a.flightCount <= 0) continue;
      const key = canonicalAirlineKey(a.icao, a.name);
      if (!key) continue;
      const prev = byKey.get(key);
      if (!prev) {
        byKey.set(key, { ...a });
      } else {
        const merged: AirlineTag = {
          // Representante = el de más vuelos; a igualdad, el que tenga icao.
          icao: prev.icao ?? a.icao,
          name:
            (a.flightCount > prev.flightCount ? a.name : prev.name) ||
            prev.name,
          flightCount: prev.flightCount + a.flightCount,
        };
        // Si la nueva variante tiene más vuelos, hereda su icao/name.
        if (a.flightCount > prev.flightCount) {
          merged.icao = a.icao ?? prev.icao;
          merged.name = a.name;
        }
        byKey.set(key, merged);
      }
    }
    // (v6.2.66) SEGUNDO merge por NOMBRE VISIBLE resuelto — colapsa variantes
    // que la clave canónica no unió (p.ej. un icao sin IATA vs el nombre
    // completo, o "DAL" vs "Delta Air Lines"). Resolvemos el nombre real con
    // `icaoToName` para que ambas caras de la misma aerolínea caigan juntas y
    // se sume su conteo (fin del chip duplicado).
    const byName = new Map<string, AirlineTag>();
    for (const a of byKey.values()) {
      const display = (icaoToName(a.icao) ?? a.name ?? "")
        .toLowerCase()
        .replace(/[^a-z0-9]/g, "");
      const nkey = display || `#${a.icao ?? a.name}`;
      const prev = byName.get(nkey);
      if (!prev) {
        byName.set(nkey, { ...a });
      } else {
        const takeNew = a.flightCount > prev.flightCount;
        byName.set(nkey, {
          icao: (takeNew ? a.icao : prev.icao) ?? prev.icao ?? a.icao,
          name: (takeNew ? a.name : prev.name) || prev.name || a.name,
          flightCount: prev.flightCount + a.flightCount,
        });
      }
    }
    return [...byName.values()].sort((x, y) => y.flightCount - x.flightCount);
  }, [airlines]);
  if (visible.length === 0) return null;
  return (
    <div
      // (v3.31.0 #15) Scroll horizontal usable: la rueda VERTICAL del
      // mouse se traduce a desplazamiento horizontal (los trackpads y el
      // touch ya hacen swipe lateral con overflow-x-auto). Antes no había
      // forma de ver los tags que se salían por la derecha con un mouse.
      onWheel={(e) => {
        if (e.deltaY !== 0) {
          e.currentTarget.scrollLeft += e.deltaY;
        }
      }}
      className="flex w-full items-center gap-2 overflow-x-auto rounded-2xl border border-slate-700/50 bg-slate-950/70 px-2.5 py-2 shadow-lg ring-1 ring-slate-800/50 backdrop-blur-md [scrollbar-width:thin]"
    >
      {/* (v4.21.0) Chips rediseñados: cada uno con borde + fondo propio
          (antes texto plano), contador en mini-pill, glow suave en el
          activo y transición de hover — armonía visual pedida. */}
      <button
        onClick={() => setSelected(null)}
        className={`shrink-0 rounded-full border px-3 py-1 text-[11px] font-semibold transition-all duration-150 ${
          selected == null
            ? "border-emerald-400/50 bg-emerald-500/20 text-emerald-100 shadow-[0_0_10px_rgba(52,211,153,0.25)]"
            : "border-slate-700/60 bg-slate-900/60 text-slate-400 hover:border-slate-500/60 hover:bg-slate-800/80 hover:text-slate-100"
        }`}
      >
        {t("fb.airline.all")}
      </button>
      <span className="h-4 w-px shrink-0 bg-slate-700/60" />
      {visible.map((a) => {
        const key = a.icao ?? a.name;
        const isActive = selected
          ? a.icao
            ? selected.icao === a.icao
            : selected.icao === null && selected.name === a.name
          : false;
        return (
          <button
            key={key}
            onClick={() =>
              setSelected(
                isActive ? null : { icao: a.icao, name: a.name },
              )
            }
            title={a.name}
            className={`group flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-medium transition-all duration-150 ${
              isActive
                ? "border-amber-400/50 bg-amber-500/20 text-amber-100 shadow-[0_0_10px_rgba(251,191,36,0.25)]"
                : "border-slate-700/60 bg-slate-900/60 text-slate-300 hover:border-slate-500/60 hover:bg-slate-800/80 hover:text-slate-100"
            }`}
          >
            {/* (v4.8.0) Mini logo de aerolínea en el chip de filtro. */}
            <AirlineLogo icao={a.icao} name={a.name} size={15} />
            <span className="font-mono font-semibold tracking-wide">
              {a.icao ?? a.name.slice(0, 3).toUpperCase()}
            </span>
            <span
              className={`rounded-full px-1.5 py-px text-[9px] font-semibold tabular-nums transition-colors ${
                isActive
                  ? "bg-amber-400/20 text-amber-200"
                  : "bg-slate-800 text-slate-400 group-hover:bg-slate-700 group-hover:text-slate-200"
              }`}
            >
              {a.flightCount}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/** (v3.6.0 Phase H — Epic E) Widget de Checklist (score breakdown).
 *  Se abre como glass overlay sobre el mapa cuando el usuario clickea
 *  el botón Checklist en la DetailActionsBar.
 *
 *  Two modes:
 *    · `expanded = false` → compact, 360x520, esquina superior derecha.
 *    · `expanded = true`  → fullsize, casi todo el mapa, scrollable.
 *
 *  Carga el report via `api.scoreGetReport(flightId)` que devuelve el
 *  persistido si existe, o lo computa al vuelo. Cada phase se muestra
 *  con su total "X/Y", expandible al clickearla para ver las reglas
 *  individuales (passed/failed) + evidencia.
 */
/** (v4.1.0) Orden cronológico canónico de las fases para el checklist.
 *  Espeja `scoring::rubric::phase_order` del backend. `general` (límites
 *  de todo el vuelo) va al final; desconocidas al fondo. */
const PHASE_ORDER: Record<string, number> = {
  pre_departure: 0,
  preflight: 0,
  pushback: 1,
  taxi_out: 2,
  takeoff: 3,
  takeoff_accel: 4,
  initial_climb: 5,
  climb: 6,
  climbing: 6,
  cruise: 7,
  descent: 8,
  initial_approach: 9,
  final_approach: 10,
  approach: 10,
  landing: 11,
  taxi_in: 12,
  landed_rollout: 12,
  arrived: 13,
  parking: 13,
  deboarding: 13,
  general: 14,
};
const phaseOrder = (p: string): number =>
  p in PHASE_ORDER ? PHASE_ORDER[p] : 99;

function ChecklistWidget({
  flightId,
  onClose,
}: {
  flightId: number;
  onClose: () => void;
}) {
  const [report, setReport] = useState<import("../lib/types").ScoreReport | null>(
    null,
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [openPhases, setOpenPhases] = useState<Set<string>>(new Set());

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    import("../lib/tauri")
      .then(({ api }) => api.scoreGetReport(flightId))
      .then((rep) => {
        if (!cancelled) {
          setReport(rep);
          setLoading(false);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setError(String(e));
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [flightId]);

  // (v4.17.0) Refresco EN VIVO: cuando el scoring del backend termina
  // (`score:done`, emitido al finalizar el vuelo o tras re-score), si el
  // report es de ESTE vuelo lo aplicamos directo del payload — sin tener
  // que cerrar/reabrir el widget ni cambiar de pantalla (bug reportado:
  // "la evaluation no aparece hasta que no cambio de pantalla").
  useEffect(() => {
    let unsub: (() => void) | null = null;
    let cancelled = false;
    import("../lib/tauri")
      .then(({ api }) =>
        api.onScoreDone((rep) => {
          if (!cancelled && rep.flightId === flightId) {
            setReport(rep);
            setLoading(false);
            setError(null);
          }
        }),
      )
      .then((u) => {
        if (cancelled) u();
        else unsub = u;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (unsub) unsub();
    };
  }, [flightId]);

  // (v4.1.0) Agrupa por fase, cuenta cumplidos/aplicables (los "na" no
  // cuentan), y ORDENA cronológicamente. Antes el render preservaba el
  // orden de `items`, que al venir persistido era alfabético → "approach"
  // arriba y "taxi" abajo. Ahora siempre va pre-departure → … → arrived.
  const byPhase = useMemo(() => {
    if (!report) return [];
    type Group = {
      phase: string;
      items: typeof report.items;
      done: number;
      applicable: number;
    };
    const groups: Group[] = [];
    for (const item of report.items) {
      let g = groups.find((x) => x.phase === item.phase);
      if (!g) {
        g = { phase: item.phase, items: [], done: 0, applicable: 0 };
        groups.push(g);
      }
      g.items.push(item);
      if (item.status === "done") {
        g.done += 1;
        g.applicable += 1;
      } else if (item.status === "missed") {
        g.applicable += 1;
      }
    }
    groups.sort((a, b) => phaseOrder(a.phase) - phaseOrder(b.phase));
    return groups;
  }, [report]);

  return (
    <div
      className={`pointer-events-auto absolute z-30 rounded-2xl border border-slate-700/80 bg-slate-950/85 shadow-2xl ring-1 ring-slate-800/70 backdrop-blur-xl transition-all ${
        expanded
          ? "inset-3 flex flex-col"
          : "right-3 top-16 flex max-h-[60vh] w-[340px] flex-col"
      }`}
    >
      <header className="flex shrink-0 items-center justify-between gap-2 border-b border-slate-800 px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <ClipboardCheck className="h-3.5 w-3.5 shrink-0 text-emerald-300" />
          <span className="truncate text-xs font-semibold text-slate-100">
            {t("fb.checklist.title")}
          </span>
          {report && (
            <span
              className={`shrink-0 rounded px-1.5 py-0.5 font-mono text-[10px] font-bold ${gradeBadgeClass(
                report.grade,
              )}`}
            >
              {Math.round(report.percentage)}% · {report.total}/{report.max}
            </span>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <button
            onClick={() => setExpanded((v) => !v)}
            title={expanded ? t("fb.checklist.shrink") : t("fb.checklist.expand")}
            className="rounded p-1 text-slate-400 hover:bg-slate-800/70 hover:text-slate-200"
          >
            {expanded ? "⤢" : "⤡"}
          </button>
          <button
            onClick={onClose}
            title={t("common.close")}
            className="rounded p-1 text-slate-400 hover:bg-slate-800/70 hover:text-slate-200"
          >
            ×
          </button>
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-2">
        {loading && (
          <div className="px-2 py-6 text-center text-xs text-slate-500">
            {t("fb.checklist.loading")}
          </div>
        )}
        {error && (
          <div className="rounded bg-rose-500/10 px-3 py-2 text-xs text-rose-300 ring-1 ring-rose-500/30">
            {error}
          </div>
        )}
        {!loading && !error && byPhase.length === 0 && (
          <div className="px-2 py-6 text-center text-xs text-slate-500">
            {t("fb.checklist.empty")}
          </div>
        )}
        {byPhase.map((group) => {
          const isOpen = openPhases.has(group.phase);
          const pct =
            group.applicable > 0 ? (group.done / group.applicable) * 100 : 100;
          const phaseColor =
            pct >= 95
              ? "text-emerald-300"
              : pct >= 70
                ? "text-amber-300"
                : "text-rose-300";
          return (
            <div
              key={group.phase}
              className="mb-1 rounded-lg border border-slate-800/70 bg-slate-900/40"
            >
              <button
                onClick={() =>
                  setOpenPhases((s) => {
                    const next = new Set(s);
                    if (next.has(group.phase)) next.delete(group.phase);
                    else next.add(group.phase);
                    return next;
                  })
                }
                className="flex w-full items-center justify-between gap-2 px-3 py-2 text-left text-[11px] font-medium text-slate-200 hover:bg-slate-800/40"
              >
                <span className="flex items-center gap-2">
                  <span className="text-[10px] text-slate-500">
                    {isOpen ? "▾" : "▸"}
                  </span>
                  <span className="capitalize">
                    {phaseLabelText(group.phase)}
                  </span>
                </span>
                <span className={`font-mono tabular-nums ${phaseColor}`}>
                  {group.done}/{group.applicable}
                </span>
              </button>
              {isOpen && (
                <ul className="space-y-1 border-t border-slate-800/70 px-3 py-2">
                  {group.items.map((it) => {
                    // (v4.1.0) Checklist puro: ✓ cumplido / ✗ no cumplido
                    // / — no aplica (datos ausentes, o no aplica a este
                    // tipo de avión). Los "—" NO penalizan el % de la fase.
                    const ruleLabel = ruleLabelText(it.ruleId, it.label);
                    const icon =
                      it.status === "done"
                        ? "✓"
                        : it.status === "missed"
                          ? "✗"
                          : "—";
                    const iconClass =
                      it.status === "done"
                        ? "text-emerald-400"
                        : it.status === "missed"
                          ? "text-rose-400"
                          : "text-slate-600";
                    return (
                      <li
                        key={it.ruleId}
                        className="flex items-baseline gap-2 text-[10px]"
                      >
                        <span
                          className={`w-3 shrink-0 text-center font-bold ${iconClass}`}
                          title={
                            it.status === "na"
                              ? t("fb.checklist.severity.skipped")
                              : undefined
                          }
                        >
                          {icon}
                        </span>
                        <span
                          className={`min-w-0 ${
                            it.status === "na"
                              ? "text-slate-500"
                              : "text-slate-300"
                          }`}
                        >
                          {ruleLabel}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** Mapea el phase id estable a un label traducido (sin diccionario por
 *  cada uno — usamos un fallback i18n con clave dinámica). */
function phaseLabelText(phase: string): string {
  const key = `fb.checklist.phase.${phase}`;
  const translated = t(key);
  if (translated === key) {
    // No traducción → fallback humano del ID.
    return phase.replace(/_/g, " ");
  }
  return translated;
}

/** (v3.7.0 Phase O) Mapea el rule_id estable a un label traducido.
 *  Si no hay traducción para el id, usamos el `fallback` (el label
 *  EN que viene del backend en `ScoreItem.label`). Esto permite que
 *  reglas viejas pre-v3.7 sigan mostrando algo razonable hasta que
 *  el flight se re-evalúe con el rubric nuevo. */
function ruleLabelText(ruleId: string, fallback: string): string {
  const key = `fb.rule.${ruleId}`;
  const translated = t(key);
  if (translated === key) {
    return fallback;
  }
  return translated;
}
