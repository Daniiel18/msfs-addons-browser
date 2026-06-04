import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import {
  Cloud,
  CloudFog,
  CloudRain,
  Cloudy,
  Eye,
  Gauge,
  Radar,
  Satellite,
  Snowflake,
  Thermometer,
  ThermometerSnowflake,
  Tornado,
  Waves,
  Wind,
  X,
  Zap,
} from "lucide-react";
import type { FlightLogEntry, WeatherSample, SimBriefBriefing } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useUnits } from "../lib/units";

/**
 * (v3.35.0) Weather modal del FlightBook — Windy embebido.
 *
 * Muestra el mapa interactivo de Windy (iframe público, sin API key)
 * centrado en el punto medio de la ruta del vuelo. El sidebar de la
 * izquierda cambia la CAPA (overlay) de Windy — satélite, radar, nubes,
 * viento, etc. — sin tener que usar los controles internos de Windy.
 *
 * Junto al mapa en vivo, el sidebar resume el clima REAL que se capturó
 * durante el vuelo (`flight_log_track`, AMBIENT del sim) y el METAR real
 * de salida/llegada tomado del OFP de SimBrief (#4) cuando el OFP actual
 * coincide con este vuelo. Ese resumen ES el clima histórico del vuelo:
 * el embed de Windy sólo expone clima ACTUAL/PRONÓSTICO, no fechas pasadas.
 *
 * (v3.35.0) Se eliminó todo el mapa MapLibre y sus capas derivadas: desde
 * v3.34.0 el iframe de Windy las tapaba por completo y `mapReady` quedaba
 * siempre en `false`, así que era código muerto que además ocultaba los
 * paneles de METAR/resumen. Ahora esos paneles viven en el sidebar.
 *
 * Resizable: handle SE, listener global de pointermove. Min 600×400.
 */

type LayerKey =
  | "satellite"
  | "radar"
  | "clouds"
  | "cbase"
  | "precip"
  | "wind"
  | "gust"
  | "temp"
  | "deg0"
  | "visibility"
  | "fog"
  | "icing"
  | "turbulence"
  | "cape"
  | "pressure";

// (v3.35.0) Cada entrada del sidebar controla una CAPA (overlay) del embed
// de Windy. `overlay` = id del overlay de Windy. Selección curada de capas
// útiles para aviación. A pedido del usuario: "Windy"→"Satelital" (overlay
// satellite) y "Engelamiento"→"Hielo" (overlay icing). Se añadieron capas
// buenas de Windy: radar, techo de nubes, ráfagas, nivel de congelación,
// niebla, turbulencia, tormentas (CAPE) y presión.
const LAYERS: { key: LayerKey; labelKey: string; Icon: typeof Cloud; overlay: string }[] = [
  { key: "satellite", labelKey: "fb.weather.layer.satellite", Icon: Satellite, overlay: "satellite" },
  { key: "radar", labelKey: "fb.weather.layer.radar", Icon: Radar, overlay: "radar" },
  { key: "clouds", labelKey: "fb.weather.layer.clouds", Icon: Cloud, overlay: "clouds" },
  { key: "cbase", labelKey: "fb.weather.layer.cbase", Icon: Cloudy, overlay: "cbase" },
  { key: "precip", labelKey: "fb.weather.layer.precip", Icon: CloudRain, overlay: "rain" },
  { key: "wind", labelKey: "fb.weather.layer.wind", Icon: Wind, overlay: "wind" },
  { key: "gust", labelKey: "fb.weather.layer.gust", Icon: Tornado, overlay: "gust" },
  { key: "temp", labelKey: "fb.weather.layer.temp", Icon: Thermometer, overlay: "temp" },
  { key: "deg0", labelKey: "fb.weather.layer.deg0", Icon: ThermometerSnowflake, overlay: "deg0" },
  { key: "visibility", labelKey: "fb.weather.layer.visibility", Icon: Eye, overlay: "visibility" },
  { key: "fog", labelKey: "fb.weather.layer.fog", Icon: CloudFog, overlay: "fog" },
  { key: "icing", labelKey: "fb.weather.layer.icing", Icon: Snowflake, overlay: "icing" },
  { key: "turbulence", labelKey: "fb.weather.layer.turbulence", Icon: Waves, overlay: "turbulence" },
  { key: "cape", labelKey: "fb.weather.layer.cape", Icon: Zap, overlay: "cape" },
  { key: "pressure", labelKey: "fb.weather.layer.pressure", Icon: Gauge, overlay: "pressure" },
];

export function WeatherModal({
  entry,
  onClose,
}: {
  entry: FlightLogEntry;
  onClose: () => void;
}) {
  const u = useUnits();
  // Visibilidad: métrica en km, imperial en millas terrestres (sm) — el
  // estándar de aviación en EE.UU. (METAR US usa SM).
  const fmtVis = (m: number): string => {
    if (u.system === "metric") {
      return m >= 9999 ? "10+ km" : `${(m / 1000).toFixed(1)} km`;
    }
    return m >= 9999 ? "6+ sm" : `${(m / 1609.34).toFixed(1)} sm`;
  };

  const [activeLayer, setActiveLayer] = useState<LayerKey>("satellite");
  const [size, setSize] = useState({ width: 880, height: 560 });
  const resizingRef = useRef<{ startX: number; startY: number; w: number; h: number } | null>(
    null,
  );

  // Vuelo activo si todavía no terminó (sin ended_at).
  const isLiveFlight = entry.endedAt == null;

  // (v4.0.0 P7.9b) Clima AMBIENT capturado durante el vuelo. Es el clima
  // histórico REAL del vuelo (el embed de Windy no muestra fechas pasadas).
  const [weather, setWeather] = useState<WeatherSample[] | null>(null);
  const hasWeather = (weather?.length ?? 0) > 0;
  useEffect(() => {
    let cancelled = false;
    api
      .getFlightWeather(entry.id)
      .then((ws) => {
        if (!cancelled) setWeather(ws);
      })
      .catch(() => setWeather([]));
    return () => {
      cancelled = true;
    };
  }, [entry.id]);

  // (v3.32.0 #4) Briefing de SimBrief (METAR/TAF reales de la vida real).
  // On-demand; se muestra SÓLO si el OFP actual coincide con origen+destino
  // de este vuelo. Falla en silencio si no hay Pilot ID configurado.
  const [briefing, setBriefing] = useState<SimBriefBriefing | null>(null);
  useEffect(() => {
    let cancelled = false;
    api
      .simbriefBriefing()
      .then((b) => {
        if (!cancelled) setBriefing(b);
      })
      .catch(() => {
        if (!cancelled) setBriefing(null);
      });
    return () => {
      cancelled = true;
    };
  }, [entry.id]);

  // (v3.33.0 #4) URL del embed de Windy centrado en el punto medio de la
  // ruta (o el origen si no hay destino). El overlay viene de la capa
  // activa del sidebar. metricWind=kt y metricTemp=°C por coherencia con
  // aviación; el `key={activeLayer}` del iframe fuerza recarga al cambiar.
  const windyUrl = (() => {
    const lat =
      entry.destinationLat != null
        ? (entry.originLat + entry.destinationLat) / 2
        : entry.originLat;
    const lon =
      entry.destinationLon != null
        ? (entry.originLon + entry.destinationLon) / 2
        : entry.originLon;
    if (!Number.isFinite(lat) || !Number.isFinite(lon)) return null;
    const p = new URLSearchParams({
      lat: lat.toFixed(3),
      lon: lon.toFixed(3),
      detailLat: lat.toFixed(3),
      detailLon: lon.toFixed(3),
      zoom: "5",
      level: "surface",
      overlay: LAYERS.find((l) => l.key === activeLayer)?.overlay ?? "wind",
      menu: "",
      message: "true",
      marker: "true",
      calendar: "now",
      type: "map",
      location: "coordinates",
      metricWind: "kt",
      metricTemp: "°C",
      radarRange: "-1",
    });
    return `https://embed.windy.com/embed2.html?${p.toString()}`;
  })();

  const briefingWx = (() => {
    if (!briefing) return null;
    const norm = (s: string | null | undefined) => (s ?? "").trim().toUpperCase();
    const matches =
      norm(briefing.originIcao) === norm(entry.originIcao) &&
      norm(briefing.destinationIcao) === norm(entry.destinationIcao) &&
      norm(entry.originIcao) !== "";
    if (!matches) return null;
    if (!briefing.origMetar && !briefing.destMetar) return null;
    return briefing;
  })();

  // (v4.0.0 P7.9b) Resumen del clima capturado para el panel lateral.
  const wxSummary = (() => {
    if (!hasWeather || !weather) return null;
    const winds = weather.map((w) => w.windSpeedKt).filter((v): v is number => v != null);
    const temps = weather.map((w) => w.oatC).filter((v): v is number => v != null);
    const baros = weather.map((w) => w.baroHpa).filter((v): v is number => v != null);
    const viss = weather.map((w) => w.visibilityM).filter((v): v is number => v != null);
    const cloudsArr = weather
      .map((w) => w.cloudCoverPct)
      .filter((v): v is number => v != null);
    const maxWind = winds.length ? Math.max(...winds) : null;
    const avgWind = winds.length
      ? Math.round(winds.reduce((a, b) => a + b, 0) / winds.length)
      : null;
    const minTemp = temps.length ? Math.round(Math.min(...temps)) : null;
    const maxTemp = temps.length ? Math.round(Math.max(...temps)) : null;
    const minVis = viss.length ? Math.round(Math.min(...viss)) : null;
    const avgCloud = cloudsArr.length
      ? Math.round(cloudsArr.reduce((a, b) => a + b, 0) / cloudsArr.length)
      : null;
    const qnh = baros.length ? baros[0] : null;
    // 2 = SIN precipitación; lluvia=4, nieve=8.
    const anyPrecip = weather.some(
      (w) => w.precipState === 4 || w.precipState === 8,
    );
    return { maxWind, avgWind, minTemp, maxTemp, minVis, avgCloud, qnh, anyPrecip };
  })();

  // Resize handlers (mismo patrón que PerformanceModal).
  useEffect(() => {
    const onMove = (ev: PointerEvent) => {
      const r = resizingRef.current;
      if (!r) return;
      const dx = ev.clientX - r.startX;
      const dy = ev.clientY - r.startY;
      const maxW = Math.floor(window.innerWidth * 0.95);
      const maxH = Math.floor(window.innerHeight * 0.95);
      setSize({
        width: Math.min(maxW, Math.max(600, r.w + dx)),
        height: Math.min(maxH, Math.max(400, r.h + dy)),
      });
    };
    const onUp = () => {
      resizingRef.current = null;
    };
    document.addEventListener("pointermove", onMove);
    document.addEventListener("pointerup", onUp);
    return () => {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("pointerup", onUp);
    };
  }, []);

  // ESC cierra.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.12 }}
      onClick={onClose}
      className="fixed inset-0 z-40 flex items-center justify-center bg-slate-950/60 backdrop-blur-sm"
    >
      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: 8 }}
        transition={{ duration: 0.16 }}
        onClick={(e) => e.stopPropagation()}
        style={{ width: size.width, height: size.height }}
        className="relative flex flex-col overflow-hidden rounded-2xl border border-slate-700/80 bg-slate-950/95 shadow-2xl ring-1 ring-slate-800/70 backdrop-blur-xl"
      >
        {/* Header */}
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-slate-800 px-4 py-3">
          <div className="flex items-center gap-2">
            <Cloud className="h-4 w-4 text-sky-300" />
            <h2 className="text-sm font-semibold text-slate-100">
              {t("fb.weather.title")} · {entry.originIcao ?? "?"} → {entry.destinationIcao ?? "?"}
            </h2>
            {isLiveFlight ? (
              <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-300 ring-1 ring-emerald-500/30">
                {t("fb.weather.live_flight")}
              </span>
            ) : (
              <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] text-slate-400">
                {t("fb.weather.past_flight")}
              </span>
            )}
          </div>
          <button
            onClick={onClose}
            title={t("common.close")}
            className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        {/* Body: sidebar (capas + clima real) + mapa Windy */}
        <div className="flex min-h-0 flex-1">
          {/* Sidebar */}
          <nav className="flex w-52 shrink-0 flex-col overflow-y-auto border-r border-slate-800 p-2 [scrollbar-width:thin]">
            {/* Selector de capas de Windy */}
            <div className="flex flex-col gap-0.5">
              {LAYERS.map((l) => (
                <button
                  key={l.key}
                  onClick={() => setActiveLayer(l.key)}
                  className={`flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] transition-colors ${
                    activeLayer === l.key
                      ? "bg-sky-500/15 text-sky-100 ring-1 ring-sky-500/40"
                      : "text-slate-300 hover:bg-slate-800/70 hover:text-slate-100"
                  }`}
                >
                  <l.Icon className="h-3.5 w-3.5 shrink-0" />
                  <span className="truncate">{t(l.labelKey)}</span>
                </button>
              ))}
            </div>

            {/* (v4.0.0 P7.9b) Resumen del clima REAL capturado en el vuelo —
                es el histórico exacto del vuelo (Windy sólo muestra
                actual/pronóstico). */}
            {wxSummary && (
              <div className="mt-3 border-t border-slate-800 pt-2.5">
                <div className="mb-1.5 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                  <Cloud className="h-3 w-3" />
                  {t("fb.weather.summary.title")}
                </div>
                <dl className="space-y-1 text-[10px] text-slate-200">
                  {wxSummary.avgCloud != null && (
                    <div className="flex justify-between gap-2">
                      <dt className="text-slate-400">{t("fb.weather.summary.clouds")}</dt>
                      <dd className="font-mono">{wxSummary.avgCloud}%</dd>
                    </div>
                  )}
                  {wxSummary.avgWind != null && (
                    <div className="flex justify-between gap-2">
                      <dt className="text-slate-400">{t("fb.weather.summary.wind_avg")}</dt>
                      <dd className="font-mono">{u.fmt.speed(wxSummary.avgWind)}</dd>
                    </div>
                  )}
                  {wxSummary.maxWind != null && (
                    <div className="flex justify-between gap-2">
                      <dt className="text-slate-400">{t("fb.weather.summary.wind_max")}</dt>
                      <dd className="font-mono">{u.fmt.speed(wxSummary.maxWind)}</dd>
                    </div>
                  )}
                  {wxSummary.minTemp != null && wxSummary.maxTemp != null && (
                    <div className="flex justify-between gap-2">
                      <dt className="text-slate-400">{t("fb.weather.summary.temp")}</dt>
                      <dd className="font-mono">
                        {Math.round(u.conv.temp(wxSummary.minTemp))}…{u.fmt.temp(wxSummary.maxTemp)}
                      </dd>
                    </div>
                  )}
                  {wxSummary.minVis != null && (
                    <div className="flex justify-between gap-2">
                      <dt className="text-slate-400">{t("fb.weather.summary.visibility")}</dt>
                      <dd className="font-mono">{fmtVis(wxSummary.minVis)}</dd>
                    </div>
                  )}
                  {wxSummary.qnh != null && (
                    <div className="flex justify-between gap-2">
                      <dt className="text-slate-400">{t("fb.weather.summary.qnh")}</dt>
                      <dd className="font-mono">{u.fmt.pressure(wxSummary.qnh)}</dd>
                    </div>
                  )}
                  <div className="flex justify-between gap-2">
                    <dt className="text-slate-400">{t("fb.weather.summary.precip")}</dt>
                    <dd className={wxSummary.anyPrecip ? "text-sky-300" : "text-slate-500"}>
                      {wxSummary.anyPrecip
                        ? t("fb.weather.summary.precip_yes")
                        : t("fb.weather.summary.precip_no")}
                    </dd>
                  </div>
                </dl>
              </div>
            )}

            {/* (v3.32.0 #4) METAR REAL de salida/llegada del OFP de SimBrief
                — sólo si el OFP coincide con este vuelo. */}
            {briefingWx && (
              <div className="mt-3 border-t border-slate-800 pt-2.5">
                <div className="mb-1.5 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                  <Cloud className="h-3 w-3" />
                  {t("fb.weather.simbrief.title")}
                </div>
                {briefingWx.origMetar && (
                  <div className="mb-2">
                    <div className="font-mono text-[10px] font-semibold text-sky-200">
                      {t("fb.weather.simbrief.dep")} {briefingWx.originIcao ?? ""}
                    </div>
                    <div className="whitespace-pre-wrap break-words font-mono text-[10px] leading-relaxed text-slate-300">
                      {briefingWx.origMetar}
                    </div>
                  </div>
                )}
                {briefingWx.destMetar && (
                  <div>
                    <div className="font-mono text-[10px] font-semibold text-sky-200">
                      {t("fb.weather.simbrief.dest")} {briefingWx.destinationIcao ?? ""}
                    </div>
                    <div className="whitespace-pre-wrap break-words font-mono text-[10px] leading-relaxed text-slate-300">
                      {briefingWx.destMetar}
                    </div>
                  </div>
                )}
              </div>
            )}
          </nav>

          {/* Mapa Windy — el iframe llena el área; el sidebar lo controla. */}
          <div className="relative min-h-0 flex-1">
            {windyUrl ? (
              <iframe
                title="Windy"
                key={activeLayer}
                src={windyUrl}
                className="absolute inset-0 h-full w-full border-0"
              />
            ) : (
              <div className="absolute inset-0 flex items-center justify-center px-6 text-center text-xs text-slate-500">
                {t("fb.weather.no_coords")}
              </div>
            )}
          </div>
        </div>

        {/* Resize handle SE */}
        <div
          onPointerDown={(ev) => {
            resizingRef.current = {
              startX: ev.clientX,
              startY: ev.clientY,
              w: size.width,
              h: size.height,
            };
            ev.preventDefault();
          }}
          className="absolute bottom-0 right-0 z-10 h-4 w-4 cursor-se-resize"
          style={{
            background:
              "linear-gradient(135deg, transparent 50%, rgba(100,116,139,0.5) 50%)",
          }}
          title={t("fb.performance.resize")}
        />
      </motion.div>
    </motion.div>
  );
}
