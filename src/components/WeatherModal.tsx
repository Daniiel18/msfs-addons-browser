import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import maplibregl from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import {
  Cloud,
  CloudRain,
  Gauge,
  MapPin,
  Thermometer,
  Wind,
  X,
} from "lucide-react";
import type {
  FlightLogEntry,
  FlightTrackPoint,
  WeatherSample,
  SimBriefBriefing,
  RouteFix,
} from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useUnits } from "../lib/units";
import { segmentTrackCoords, smoothCatmullRom } from "../lib/smooth";

/**
 * (v4.16.0 #5a) Weather modal del FlightBook — mapa MapLibre con capas
 * meteo de OpenWeather + la RUTA del vuelo (waypoints del navlog SimBrief)
 * trazada encima. Reemplaza el iframe de Windy.
 *
 * El sidebar izquierdo cambia la CAPA meteo (nubes / precipitación / viento
 * / temperatura / presión) que se pinta como tiles raster sobre el basemap
 * oscuro (CARTO). La ruta planificada se dibuja como línea + waypoints, con
 * marcadores de origen/destino. (NO se dibuja una línea recta origen→destino
 * cuando no hay waypoints — se evita el artefacto de "track recto".)
 *
 * El sidebar también resume el clima REAL capturado durante el vuelo
 * (`flight_log_track`, AMBIENT del sim) y el METAR real de salida/llegada
 * del OFP de SimBrief cuando coincide con este vuelo.
 *
 * Resizable: handle SE, listener global de pointermove. Min 600×400.
 */

type LayerKey = "none" | "clouds" | "precip" | "wind" | "temp" | "pressure";

// (v4.16.0 #5a) Capas meteo de OpenWeather (tiles raster). `owLayer` = id de
// la capa en la API de tiles de OpenWeather (https://tile.openweathermap.org
// /map/{owLayer}/{z}/{x}/{y}.png). "none" = solo basemap. Reutilizamos las
// claves i18n existentes de capas donde aplican.
const LAYERS: {
  key: LayerKey;
  labelKey: string;
  Icon: typeof Cloud;
  owLayer: string | null;
}[] = [
  { key: "none", labelKey: "fb.weather.layer.none", Icon: MapPin, owLayer: null },
  { key: "clouds", labelKey: "fb.weather.layer.clouds", Icon: Cloud, owLayer: "clouds_new" },
  { key: "precip", labelKey: "fb.weather.layer.precip", Icon: CloudRain, owLayer: "precipitation_new" },
  { key: "wind", labelKey: "fb.weather.layer.wind", Icon: Wind, owLayer: "wind_new" },
  { key: "temp", labelKey: "fb.weather.layer.temp", Icon: Thermometer, owLayer: "temp_new" },
  { key: "pressure", labelKey: "fb.weather.layer.pressure", Icon: Gauge, owLayer: "pressure_new" },
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

  const [activeLayer, setActiveLayer] = useState<LayerKey>("clouds");
  const [size, setSize] = useState({ width: 880, height: 560 });
  const resizingRef = useRef<{ startX: number; startY: number; w: number; h: number } | null>(
    null,
  );

  // (v4.16.0 #5a) Mapa MapLibre + key de OpenWeather + waypoints de la ruta.
  const mapContainerRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  const [owKey, setOwKey] = useState<string | null>(null);
  const [routeFixes, setRouteFixes] = useState<RouteFix[]>([]);
  // Contador que incrementa cuando el mapa terminó de cargar — dispara los
  // effects de datos/capa (que dependen de `mapRef.current` ya listo).
  const [mapReady, setMapReady] = useState(0);

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

  // (v4.16.0 #5a) Carga la key de OpenWeather + los waypoints del navlog
  // (ruta planificada de SimBrief pegada al vuelo).
  useEffect(() => {
    let cancelled = false;
    api
      .getOpenweatherKey()
      .then((k) => {
        if (!cancelled) setOwKey(k);
      })
      .catch(() => {});
    api
      .flightRouteFixes(entry.id)
      .then((fx) => {
        if (!cancelled) setRouteFixes(fx);
      })
      .catch(() => setRouteFixes([]));
    return () => {
      cancelled = true;
    };
  }, [entry.id]);

  // (v4.19.0) TRACK REAL del vuelo — el mismo trazado del mapa de rutas
  // (feedback usuario: el mapa del weather debe mostrar exactamente lo
  // que se voló, no la ruta planificada). Si el vuelo tiene track, se
  // dibuja ese; los fixes del navlog quedan como fallback.
  const [trackPoints, setTrackPoints] = useState<FlightTrackPoint[]>([]);
  useEffect(() => {
    let cancelled = false;
    setTrackPoints([]);
    api
      .getFlightTrack(entry.id)
      .then((pts) => {
        if (!cancelled) setTrackPoints(pts);
      })
      .catch(() => setTrackPoints([]));
    return () => {
      cancelled = true;
    };
  }, [entry.id]);

  // (v4.17.1) Coordenada válida = finita Y lejos de (0,0). Algunos vuelos
  // guardan el destino con coords basura ≈(0,0) (lookup fallido) — eso
  // ponía un marcador fantasma frente a África y el fitBounds encuadraba
  // medio planeta (bug reportado con KSEA→PAJN).
  const coordOk = (lat: number | null | undefined, lon: number | null | undefined) =>
    lat != null &&
    lon != null &&
    Number.isFinite(lat) &&
    Number.isFinite(lon) &&
    !(Math.abs(lat) < 0.05 && Math.abs(lon) < 0.05);
  const originOk = coordOk(entry.originLat, entry.originLon);
  const destOk = coordOk(entry.destinationLat, entry.destinationLon);

  // (v4.16.0 #5a) Inicializa el mapa MapLibre UNA vez (basemap oscuro CARTO).
  // Sources/layers de la ruta se crean vacíos; los effects de abajo hacen
  // setData + swap de la capa meteo.
  useEffect(() => {
    if (mapRef.current || !mapContainerRef.current) return;
    const lon0 = originOk ? entry.originLon : 0;
    const lat0 = originOk ? entry.originLat : 25;
    const style: maplibregl.StyleSpecification = {
      version: 8,
      sources: {
        "carto-dark": {
          type: "raster",
          tiles: [
            "https://cartodb-basemaps-a.global.ssl.fastly.net/dark_all/{z}/{x}/{y}.png",
            "https://cartodb-basemaps-b.global.ssl.fastly.net/dark_all/{z}/{x}/{y}.png",
            "https://cartodb-basemaps-c.global.ssl.fastly.net/dark_all/{z}/{x}/{y}.png",
          ],
          tileSize: 256,
          attribution: "© OpenStreetMap · © CARTO · Weather © OpenWeather",
        },
      },
      layers: [{ id: "carto-dark-layer", type: "raster", source: "carto-dark" }],
    };
    const map = new maplibregl.Map({
      container: mapContainerRef.current,
      style,
      center: [lon0, lat0],
      zoom: 4,
      attributionControl: false,
    });
    map.addControl(new maplibregl.AttributionControl({ compact: true }), "bottom-right");
    const empty: GeoJSON.FeatureCollection = { type: "FeatureCollection", features: [] };
    map.on("load", () => {
      // Ruta planificada (violeta) + waypoints + endpoints.
      map.addSource("wx-route", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-route-line",
        type: "line",
        source: "wx-route",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": "#a78bfa", "line-width": 2.2, "line-opacity": 0.95 },
      });
      map.addSource("wx-fixes", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-fixes-circle",
        type: "circle",
        source: "wx-fixes",
        paint: {
          "circle-radius": 2.6,
          "circle-color": "#c4b5fd",
          "circle-stroke-width": 1,
          "circle-stroke-color": "#1e1b4b",
        },
      });
      map.addSource("wx-ends", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-ends-circle",
        type: "circle",
        source: "wx-ends",
        paint: {
          "circle-radius": 5,
          "circle-color": [
            "case",
            ["==", ["get", "kind"], "orig"],
            "#34d399",
            "#fb7185",
          ],
          "circle-stroke-width": 1.5,
          "circle-stroke-color": "#0f172a",
        },
      });
      mapRef.current = map;
      // Disparar el primer render de datos/capa.
      setMapReady((n) => n + 1);
    });
    return () => {
      map.remove();
      mapRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // (v4.16.0 #5a) Ruta (línea + waypoints + endpoints) + encuadre.
  useEffect(() => {
    const map = mapRef.current;
    // (v4.17.1) Solo exigimos que el mapa exista: `mapRef` se asigna DENTRO
    // de map.on("load"), tras crear los sources — así que si no es null,
    // los sources existen y setData/fitBounds funcionan. El guard anterior
    // `isStyleLoaded()` devolvía false mientras cargaban tiles y el effect
    // hacía return SIN reintento → la ruta/marcadores no se dibujaban nunca
    // (bug reportado: "no me está trazando la ruta").
    if (!map) return;
    // (v4.19.0) TRACK REAL primero — mismo pipeline que el mapa de rutas
    // (segmentación por gaps + Catmull-Rom). Si el vuelo no tiene track
    // (sin datos), caemos a la ruta planificada del navlog como antes.
    const trackLines = segmentTrackCoords(
      trackPoints.map((p) => ({ lon: p.lon, lat: p.lat, ts: p.ts })),
    )
      .filter((seg) => seg.length >= 2)
      .map((seg) => smoothCatmullRom(seg, 8));
    const hasTrack = trackLines.length > 0;

    const coords: [number, number][] = [];
    if (hasTrack) {
      for (const seg of trackLines) {
        for (const c of seg) coords.push(c as [number, number]);
      }
    } else {
      if (originOk) coords.push([entry.originLon, entry.originLat]);
      for (const f of routeFixes) {
        if (coordOk(f.lat, f.lon)) coords.push([f.lon, f.lat]);
      }
      if (destOk) {
        coords.push([entry.destinationLon as number, entry.destinationLat as number]);
      }
    }
    // Línea: track real (MultiLineString, ámbar como el routemap) o ruta
    // planificada SOLO si hay waypoints — NUNCA recta origen→destino.
    const lineFeats: GeoJSON.Feature[] = hasTrack
      ? [
          {
            type: "Feature",
            properties: { real: true },
            geometry: { type: "MultiLineString", coordinates: trackLines },
          },
        ]
      : routeFixes.length > 0 && coords.length >= 2
        ? [
            {
              type: "Feature",
              properties: { real: false },
              geometry: { type: "LineString", coordinates: coords },
            },
          ]
        : [];
    (map.getSource("wx-route") as maplibregl.GeoJSONSource | undefined)?.setData({
      type: "FeatureCollection",
      features: lineFeats,
    });
    // Color según lo que se dibuja: ámbar = track volado (igual que el
    // mapa de rutas), violeta = ruta planificada.
    if (map.getLayer("wx-route-line")) {
      map.setPaintProperty(
        "wx-route-line",
        "line-color",
        hasTrack ? "#fbbf24" : "#a78bfa",
      );
    }
    (map.getSource("wx-fixes") as maplibregl.GeoJSONSource | undefined)?.setData({
      type: "FeatureCollection",
      features: hasTrack
        ? []
        : routeFixes
            .filter((f) => coordOk(f.lat, f.lon))
            .map((f) => ({
              type: "Feature",
              properties: { ident: f.ident },
              geometry: { type: "Point", coordinates: [f.lon, f.lat] },
            })),
    });
    const ends: GeoJSON.Feature[] = [];
    if (originOk) {
      ends.push({
        type: "Feature",
        properties: { kind: "orig", icao: entry.originIcao ?? "" },
        geometry: { type: "Point", coordinates: [entry.originLon, entry.originLat] },
      });
    }
    if (destOk) {
      ends.push({
        type: "Feature",
        properties: { kind: "dest", icao: entry.destinationIcao ?? "" },
        geometry: {
          type: "Point",
          coordinates: [entry.destinationLon as number, entry.destinationLat as number],
        },
      });
    }
    (map.getSource("wx-ends") as maplibregl.GeoJSONSource | undefined)?.setData({
      type: "FeatureCollection",
      features: ends,
    });
    if (coords.length >= 2) {
      const lons = coords.map((c) => c[0]);
      const lats = coords.map((c) => c[1]);
      map.fitBounds(
        [
          [Math.min(...lons), Math.min(...lats)],
          [Math.max(...lons), Math.max(...lats)],
        ],
        { padding: 60, duration: 600, maxZoom: 7 },
      );
    } else if (coords.length === 1) {
      map.easeTo({ center: coords[0], zoom: 6, duration: 400 });
    }
  }, [mapReady, routeFixes, trackPoints, entry.id, originOk, destOk]);

  // (v4.16.0 #5a) Capa meteo OpenWeather — añade/quita el raster según la
  // capa activa + la key. Se inserta DEBAJO de la ruta para no taparla.
  useEffect(() => {
    const map = mapRef.current;
    // (v4.17.1) Mismo fix que la ruta: sin `isStyleLoaded()` — bloqueaba
    // la capa meteo inicial mientras cargaban tiles, sin reintento.
    if (!map) return;
    if (map.getLayer("wx-owm-layer")) map.removeLayer("wx-owm-layer");
    if (map.getSource("wx-owm")) map.removeSource("wx-owm");
    const def = LAYERS.find((l) => l.key === activeLayer);
    if (!def?.owLayer || !owKey) return;
    map.addSource("wx-owm", {
      type: "raster",
      tiles: [
        `https://tile.openweathermap.org/map/${def.owLayer}/{z}/{x}/{y}.png?appid=${owKey}`,
      ],
      tileSize: 256,
    });
    const beforeId = map.getLayer("wx-route-line") ? "wx-route-line" : undefined;
    map.addLayer(
      { id: "wx-owm-layer", type: "raster", source: "wx-owm", paint: { "raster-opacity": 0.75 } },
      beforeId,
    );
  }, [mapReady, activeLayer, owKey]);

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

  // (v4.16.1 #5b → v4.19.0) METAR a mostrar: SIEMPRE preferimos el METAR
  // PERSISTIDO del vuelo (histórico, inmutable). El briefing live solo se
  // usa cuando el vuelo todavía no tiene METAR guardado. Antes era al
  // revés y un vuelo viejo de la misma ruta mostraba el clima de HOY —
  // y al hacer otro vuelo el METAR "desaparecía" (bug reportado).
  const metarView = (() => {
    if (entry.metarOrigin || entry.metarDest) {
      return {
        origMetar: entry.metarOrigin ?? null,
        destMetar: entry.metarDest ?? null,
        originIcao: entry.originIcao,
        destinationIcao: entry.destinationIcao,
      };
    }
    if (briefingWx && (briefingWx.origMetar || briefingWx.destMetar)) {
      return {
        origMetar: briefingWx.origMetar ?? null,
        destMetar: briefingWx.destMetar ?? null,
        originIcao: briefingWx.originIcao ?? entry.originIcao,
        destinationIcao: briefingWx.destinationIcao ?? entry.destinationIcao,
      };
    }
    return null;
  })();

  // (v4.19.0) PERSISTENCIA del METAR: si el briefing live coincide con
  // este vuelo y el vuelo no tiene METAR guardado, lo grabamos en la DB
  // en ese momento — así queda para siempre aunque el OFP cambie con el
  // próximo vuelo. Solo rellena NULLs (nunca pisa lo capturado al
  // finalizar el vuelo). Best-effort: fallo silencioso.
  const savedMetarRef = useRef(false);
  useEffect(() => {
    savedMetarRef.current = false;
  }, [entry.id]);
  useEffect(() => {
    if (savedMetarRef.current) return;
    if (entry.metarOrigin && entry.metarDest) return;
    if (!briefingWx || (!briefingWx.origMetar && !briefingWx.destMetar)) return;
    savedMetarRef.current = true;
    api
      .saveFlightMetar(entry.id, briefingWx.origMetar ?? null, briefingWx.destMetar ?? null)
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entry.id, briefingWx]);

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

            {/* (v3.32.0 #4 / v4.16.1 #5b) METAR REAL de salida/llegada — del
                briefing live del OFP si coincide, o del METAR persistido del
                vuelo (consulta histórica). */}
            {metarView && (
              <div className="mt-3 border-t border-slate-800 pt-2.5">
                <div className="mb-1.5 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                  <Cloud className="h-3 w-3" />
                  {t("fb.weather.simbrief.title")}
                </div>
                {metarView.origMetar && (
                  <div className="mb-2">
                    <div className="font-mono text-[10px] font-semibold text-sky-200">
                      {t("fb.weather.simbrief.dep")} {metarView.originIcao ?? ""}
                    </div>
                    <div className="whitespace-pre-wrap break-words font-mono text-[10px] leading-relaxed text-slate-300">
                      {metarView.origMetar}
                    </div>
                  </div>
                )}
                {metarView.destMetar && (
                  <div>
                    <div className="font-mono text-[10px] font-semibold text-sky-200">
                      {t("fb.weather.simbrief.dest")} {metarView.destinationIcao ?? ""}
                    </div>
                    <div className="whitespace-pre-wrap break-words font-mono text-[10px] leading-relaxed text-slate-300">
                      {metarView.destMetar}
                    </div>
                  </div>
                )}
              </div>
            )}
          </nav>

          {/* (v4.16.0 #5a) Mapa MapLibre — basemap oscuro + capa meteo
              OpenWeather + ruta del vuelo. El sidebar cambia la capa. */}
          <div className="relative min-h-0 flex-1">
            <div ref={mapContainerRef} className="absolute inset-0 h-full w-full" />
            {owKey === null && (
              <div className="pointer-events-none absolute bottom-2 left-2 z-10 rounded bg-slate-900/85 px-2 py-1 text-[10px] text-amber-300 ring-1 ring-amber-500/30">
                {t("fb.weather.no_owkey")}
              </div>
            )}
            {!originOk && (
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
