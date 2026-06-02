import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import maplibregl, { type GeoJSONSource } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import {
  Cloud,
  CloudRain,
  CloudSnow,
  CloudLightning,
  Loader2,
  Snowflake,
  Wind,
  X,
  Zap,
} from "lucide-react";
import type { FlightLogEntry, WeatherSample } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { buildTerminatorPolygon } from "../lib/terminator";

/** (v4.0.0 P7.9b) Destino de una "barba" de viento: dado un punto y
 *  el viento (dir DESDE donde sopla + velocidad), calcula el extremo
 *  de una flecha corta que apunta HACIA donde va el viento. La
 *  longitud escala suave con la velocidad (clamp 5..60 kt → 0.04..0.30°). */
function windArrowEnd(
  lon: number,
  lat: number,
  fromDir: number,
  speedKt: number,
): [number, number] {
  // El viento "viene de" fromDir; la flecha apunta hacia (fromDir+180).
  const toDir = (fromDir + 180) % 360;
  const rad = (toDir * Math.PI) / 180;
  const len = 0.04 + Math.min(Math.max(speedKt, 5), 60) * 0.0043;
  // Corrección de latitud para que la flecha se vea proporcional.
  const dLat = Math.cos(rad) * len;
  const dLon = (Math.sin(rad) * len) / Math.max(0.2, Math.cos((lat * Math.PI) / 180));
  return [lon + dLon, lat + dLat];
}

/**
 * (v4.0.0 — P7 iter 1) Weather modal del FlightBook.
 *
 * Modal floating resizable inspirado en Windy. Muestra la ruta del
 * vuelo seleccionado sobre un mapa MapLibre con globo 3D y permite
 * activar capas meteorológicas de aviación.
 *
 * ## Estrategia de datos
 *
 * Para vuelos **activos** (sin `ended_at`), se muestran tiles LIVE
 * de RainViewer (clouds satelitales + radar de precipitación). Es
 * la data que el piloto ve "ahora mismo" mientras está volando —
 * útil para validar SIGMET en ruta.
 *
 * Para vuelos **pasados** (`ended_at` != null), se muestra un
 * mensaje "Histórico no disponible" porque las APIs gratuitas no
 * soportan query histórica de weather radar. La solución correcta
 * es persistir weather por sample en `flight_log_track` durante el
 * vuelo activo — pendiente para próxima iteración v4.x.
 *
 * ## Capas implementadas en esta iteración
 *
 *   · **Clouds** — RainViewer satellite tiles (raster, todos los
 *     niveles de zoom). Tile URL público sin API key.
 *   · **Precipitation** — RainViewer radar tiles (raster, idem).
 *
 * Otras capas (Wind particles, SIGMET, Turbulence, Icing, Jet
 * Stream, Tropopause) están como tabs con "Próximamente". Se
 * habilitarán en iteraciones P7.2..P7.5 a medida que sumamos las
 * fuentes de datos correspondientes (OpenWeatherMap wind grids,
 * AviationWeather.gov SIGMET GeoJSON, etc.).
 *
 * ## Resizable
 *
 * Mismo patrón que PerformanceModal: handle SE absoluto, listener
 * global de pointermove. Min 600×400, max 95% viewport.
 */
export function WeatherModal({
  entry,
  onClose,
}: {
  entry: FlightLogEntry;
  onClose: () => void;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  const [mapReady, setMapReady] = useState(false);
  const [activeLayer, setActiveLayer] = useState<LayerKey>("clouds");
  const [size, setSize] = useState({ width: 880, height: 560 });
  const resizingRef = useRef<{ startX: number; startY: number; w: number; h: number } | null>(
    null,
  );

  // Vuelo activo si todavía no terminó (sin ended_at).
  const isLiveFlight = entry.endedAt == null;

  // (v4.0.0 P7.9b) Weather samples capturados durante el vuelo.
  const [weather, setWeather] = useState<WeatherSample[] | null>(null);
  const hasWeather = (weather?.length ?? 0) > 0;

  // Carga el track del vuelo para dibujar la ruta en el mapa.
  const [trackCoords, setTrackCoords] = useState<[number, number][] | null>(
    null,
  );
  useEffect(() => {
    let cancelled = false;
    api
      .getFlightTrack(entry.id)
      .then((pts) => {
        if (cancelled) return;
        const coords = pts
          .filter((p) => Number.isFinite(p.lat) && Number.isFinite(p.lon))
          .map((p) => [p.lon, p.lat] as [number, number]);
        setTrackCoords(coords);
      })
      .catch(() => setTrackCoords([]));
    return () => {
      cancelled = true;
    };
  }, [entry.id]);

  // (v4.0.0 P7.9b) Carga los weather samples del vuelo.
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

  // Inicializa el mapa una sola vez.
  useEffect(() => {
    if (!containerRef.current || mapRef.current) return;
    const map = new maplibregl.Map({
      container: containerRef.current,
      style: {
        version: 8,
        sources: {
          "carto-dark": {
            type: "raster",
            tiles: [
              "https://a.basemaps.cartocdn.com/rastertiles/voyager_nolabels/{z}/{x}/{y}.png",
              "https://b.basemaps.cartocdn.com/rastertiles/voyager_nolabels/{z}/{x}/{y}.png",
            ],
            tileSize: 256,
            attribution: "© OpenStreetMap · © CARTO",
          },
        },
        layers: [
          {
            id: "basemap",
            type: "raster",
            source: "carto-dark",
          },
        ],
        glyphs: "https://demotiles.maplibre.org/font/{fontstack}/{range}.pbf",
      },
      center: [
        ((entry.originLon ?? 0) + (entry.destinationLon ?? 0)) / 2,
        ((entry.originLat ?? 0) + (entry.destinationLat ?? 0)) / 2,
      ],
      zoom: 3,
      attributionControl: { compact: true },
    });

    // Globe projection — aplicar después de style.load.
    const applyGlobe = () => {
      try {
        map.setProjection({ type: "globe" });
      } catch {
        // ignore — projection puede no estar disponible
      }
    };
    map.on("style.load", applyGlobe);

    // Setup de sources/layers UNA vez al primer load.
    map.once("load", () => {
      applyGlobe();
      const empty: GeoJSON.FeatureCollection = {
        type: "FeatureCollection",
        features: [],
      };

      // Ruta del vuelo (verde esmeralda con halo blanco — mismo
      // estilo que el RoutesMapView para consistencia visual).
      map.addSource("wx-route", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-route-halo",
        type: "line",
        source: "wx-route",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "rgba(255, 255, 255, 0.6)",
          "line-width": 4,
        },
      });
      map.addLayer({
        id: "wx-route-line",
        type: "line",
        source: "wx-route",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          // (v4.0.0 P7.9b iter2) Ruta en ÁMBAR para distinguirla
          // claramente de las barbas de viento (tonos fríos). Antes
          // era verde y se confundía con el viento suave (celeste/verde).
          "line-color": "#f59e0b",
          "line-width": 3,
        },
      });

      // Endpoints (origen + destino).
      map.addSource("wx-endpoints", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-endpoints-dot",
        type: "circle",
        source: "wx-endpoints",
        paint: {
          "circle-radius": 6,
          "circle-color": "#fbbf24",
          "circle-stroke-width": 2,
          "circle-stroke-color": "rgba(255, 255, 255, 0.9)",
        },
      });

      // Terminator día/noche (mismo patrón que RoutesMapView).
      map.addSource("wx-terminator", {
        type: "geojson",
        data: buildTerminatorPolygon(new Date()),
      });
      map.addLayer({
        id: "wx-terminator-shadow",
        type: "fill",
        source: "wx-terminator",
        paint: {
          "fill-color": "#020617",
          "fill-opacity": 0.4,
          "fill-antialias": true,
        },
      });

      // === Capas weather (raster sources, alternativas) ===
      // RainViewer satellite tiles para clouds. URL pública con
      // timestamp `0` (última imagen disponible). Sin API key.
      map.addSource("wx-clouds", {
        type: "raster",
        tiles: [
          "https://tilecache.rainviewer.com/v2/satellite/0/256/{z}/{x}/{y}/0/0_0.png",
        ],
        tileSize: 256,
        attribution: "© RainViewer",
      });
      map.addLayer({
        id: "wx-clouds-layer",
        type: "raster",
        source: "wx-clouds",
        paint: {
          "raster-opacity": 0.7,
        },
        layout: { visibility: "visible" },
      });

      // (v4.0.0 P7.9b) Barbas de viento — LineStrings calculadas de
      // los weather samples. Color por velocidad (azul→rojo). Una
      // capa de halo + línea + cabeza de flecha (circle en el extremo).
      map.addSource("wx-wind", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-wind-line",
        type: "line",
        source: "wx-wind",
        layout: { "line-cap": "round", visibility: "none" },
        paint: {
          "line-width": 1.6,
          "line-color": [
            "interpolate",
            ["linear"],
            ["get", "speed"],
            5, "#38bdf8", // celeste — calmo
            20, "#22c55e", // verde — moderado
            35, "#facc15", // amarillo — fuerte
            50, "#ef4444", // rojo — muy fuerte
          ],
        },
      });
      map.addLayer({
        id: "wx-wind-head",
        type: "circle",
        source: "wx-wind",
        filter: ["==", ["geometry-type"], "Point"],
        layout: { visibility: "none" },
        paint: {
          "circle-radius": 2.4,
          "circle-color": [
            "interpolate",
            ["linear"],
            ["get", "speed"],
            5, "#38bdf8",
            20, "#22c55e",
            35, "#facc15",
            50, "#ef4444",
          ],
        },
      });

      // RainViewer radar tiles para precipitación.
      map.addSource("wx-precip", {
        type: "raster",
        tiles: [
          "https://tilecache.rainviewer.com/v2/radar/nowcast_0/256/{z}/{x}/{y}/2/1_1.png",
        ],
        tileSize: 256,
        attribution: "© RainViewer",
      });
      map.addLayer({
        id: "wx-precip-layer",
        type: "raster",
        source: "wx-precip",
        paint: {
          "raster-opacity": 0.65,
        },
        layout: { visibility: "none" },
      });

      setMapReady(true);
    });

    mapRef.current = map;
    return () => {
      map.off("style.load", applyGlobe);
      map.remove();
      mapRef.current = null;
      setMapReady(false);
    };
  }, [entry.originLat, entry.originLon, entry.destinationLat, entry.destinationLon]);

  // Cuando llegan los puntos del track, los pintamos y ajustamos
  // el viewport al bounding box de la ruta.
  useEffect(() => {
    if (!mapReady || !trackCoords) return;
    const map = mapRef.current;
    if (!map) return;
    try {
      const routeSource = map.getSource("wx-route") as GeoJSONSource | undefined;
      const endpointsSource = map.getSource("wx-endpoints") as
        | GeoJSONSource
        | undefined;
      if (!routeSource || !endpointsSource) return;

      if (trackCoords.length > 1) {
        routeSource.setData({
          type: "Feature",
          properties: {},
          geometry: {
            type: "LineString",
            coordinates: trackCoords,
          },
        });
      }
      // Endpoints siempre se pintan (orig/dest del entry), incluso
      // si no hay track interpolado.
      const endpoints: GeoJSON.Feature<GeoJSON.Point>[] = [];
      if (entry.originLat != null && entry.originLon != null) {
        endpoints.push({
          type: "Feature",
          properties: { kind: "origin" },
          geometry: { type: "Point", coordinates: [entry.originLon, entry.originLat] },
        });
      }
      if (entry.destinationLat != null && entry.destinationLon != null) {
        endpoints.push({
          type: "Feature",
          properties: { kind: "destination" },
          geometry: {
            type: "Point",
            coordinates: [entry.destinationLon, entry.destinationLat],
          },
        });
      }
      endpointsSource.setData({
        type: "FeatureCollection",
        features: endpoints,
      });

      // Encuadrar a la ruta.
      const bounds = new maplibregl.LngLatBounds();
      if (trackCoords.length > 0) {
        trackCoords.forEach((c) => bounds.extend(c));
      } else {
        if (entry.originLat != null && entry.originLon != null) {
          bounds.extend([entry.originLon, entry.originLat]);
        }
        if (entry.destinationLat != null && entry.destinationLon != null) {
          bounds.extend([entry.destinationLon, entry.destinationLat]);
        }
      }
      if (!bounds.isEmpty()) {
        map.fitBounds(bounds, { padding: 80, maxZoom: 6, duration: 800 });
      }
    } catch (e) {
      console.warn("[WeatherModal] update sources failed:", e);
    }
  }, [
    mapReady,
    trackCoords,
    entry.originLat,
    entry.originLon,
    entry.destinationLat,
    entry.destinationLon,
  ]);

  // (v4.0.0 P7.9b) Cuando llegan los weather samples, generamos las
  // barbas de viento (una flecha cada N samples para no saturar) y
  // las cargamos en la source wx-wind.
  useEffect(() => {
    if (!mapReady || !weather) return;
    const map = mapRef.current;
    if (!map) return;
    const src = map.getSource("wx-wind") as GeoJSONSource | undefined;
    if (!src) return;
    const feats: GeoJSON.Feature[] = [];
    // Muestreamos hasta ~40 barbas a lo largo del vuelo.
    const withWind = weather.filter(
      (w) => w.windSpeedKt != null && w.windDirDeg != null,
    );
    const step = Math.max(1, Math.floor(withWind.length / 40));
    for (let i = 0; i < withWind.length; i += step) {
      const w = withWind[i];
      const speed = w.windSpeedKt ?? 0;
      const dir = w.windDirDeg ?? 0;
      const end = windArrowEnd(w.lon, w.lat, dir, speed);
      feats.push({
        type: "Feature",
        properties: { speed },
        geometry: { type: "LineString", coordinates: [[w.lon, w.lat], end] },
      });
      feats.push({
        type: "Feature",
        properties: { speed },
        geometry: { type: "Point", coordinates: end },
      });
    }
    src.setData({ type: "FeatureCollection", features: feats });
  }, [weather, mapReady]);

  // Cambia qué capa raster está visible según el tab activo.
  useEffect(() => {
    if (!mapReady) return;
    const map = mapRef.current;
    if (!map) return;
    map.setLayoutProperty(
      "wx-clouds-layer",
      "visibility",
      activeLayer === "clouds" ? "visible" : "none",
    );
    map.setLayoutProperty(
      "wx-precip-layer",
      "visibility",
      activeLayer === "precip" ? "visible" : "none",
    );
    const windVis = activeLayer === "wind" ? "visible" : "none";
    map.setLayoutProperty("wx-wind-line", "visibility", windVis);
    map.setLayoutProperty("wx-wind-head", "visibility", windVis);
  }, [activeLayer, mapReady]);

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

  // Cuando el modal cambia de tamaño, re-disparamos resize del mapa
  // (Maplibre necesita saber que su canvas cambió de dimensiones).
  useEffect(() => {
    if (!mapReady) return;
    const map = mapRef.current;
    if (!map) return;
    map.resize();
  }, [size.width, size.height, mapReady]);

  const layerAvailable = (key: LayerKey): boolean => {
    // (v4.0.0 P7.9b) Wind disponible si el vuelo capturó weather
    // (live o pasado). Clouds/precip siguen siendo LIVE-only (tiles
    // RainViewer no tienen histórico).
    if (key === "wind") return hasWeather;
    if (key === "clouds" || key === "precip") return isLiveFlight;
    return false; // tropopause/sigmet/turbulence/icing/jetstream → P7.10+
  };

  // (v4.0.0 P7.9b) Para vuelos pasados con weather, auto-seleccionamos
  // la tab Wind (la única con data histórica). Para vuelos live se
  // queda en clouds (RainViewer).
  useEffect(() => {
    if (!isLiveFlight && hasWeather) setActiveLayer("wind");
  }, [isLiveFlight, hasWeather]);

  // (v4.0.0 P7.9b) Resumen de weather para el panel lateral.
  const wxSummary = (() => {
    if (!hasWeather || !weather) return null;
    const winds = weather.map((w) => w.windSpeedKt).filter((v): v is number => v != null);
    const temps = weather.map((w) => w.oatC).filter((v): v is number => v != null);
    const baros = weather.map((w) => w.baroHpa).filter((v): v is number => v != null);
    const maxWind = winds.length ? Math.max(...winds) : null;
    const avgWind = winds.length
      ? Math.round(winds.reduce((a, b) => a + b, 0) / winds.length)
      : null;
    const minTemp = temps.length ? Math.round(Math.min(...temps)) : null;
    const maxTemp = temps.length ? Math.round(Math.max(...temps)) : null;
    const qnh = baros.length ? baros[0] : null;
    const anyPrecip = weather.some((w) => (w.precipState ?? 0) >= 2);
    return { maxWind, avgWind, minTemp, maxTemp, qnh, anyPrecip };
  })();

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

        {/* Body: tabs sidebar + map */}
        <div className="flex min-h-0 flex-1">
          {/* Tabs sidebar */}
          <nav className="flex w-40 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-slate-800 p-2">
            {LAYERS.map((l) => {
              const available = layerAvailable(l.key);
              return (
                <button
                  key={l.key}
                  onClick={() => available && setActiveLayer(l.key)}
                  disabled={!available}
                  title={
                    available
                      ? undefined
                      : isLiveFlight
                        ? t("fb.weather.coming_soon")
                        : t("fb.weather.no_historical")
                  }
                  className={`flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-[11px] transition-colors ${
                    activeLayer === l.key
                      ? "bg-sky-500/15 text-sky-100 ring-1 ring-sky-500/40"
                      : available
                        ? "text-slate-300 hover:bg-slate-800/70 hover:text-slate-100"
                        : "text-slate-600 opacity-50"
                  }`}
                >
                  <l.Icon className="h-3.5 w-3.5" />
                  <span className="truncate">{t(l.labelKey)}</span>
                </button>
              );
            })}
          </nav>

          {/* Map area */}
          <div className="relative min-h-0 flex-1">
            <div ref={containerRef} className="absolute inset-0" />
            {!mapReady && (
              <div className="absolute inset-0 flex items-center justify-center bg-slate-950/60 text-xs text-slate-400">
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                {t("fb.weather.loading_map")}
              </div>
            )}
            {/* (v4.0.0 P7.9b) Banner "histórico no disponible" SOLO si
                el vuelo pasado NO capturó weather (pre-v3.16.0 / VAS). */}
            {!isLiveFlight && !hasWeather && (
              <div className="absolute right-3 top-3 max-w-xs rounded-lg bg-amber-500/10 px-3 py-2 text-[11px] text-amber-200 ring-1 ring-amber-500/30 backdrop-blur">
                {t("fb.weather.historical_unavailable")}
              </div>
            )}

            {/* (v4.0.0 P7.9b) Panel resumen de weather del vuelo. */}
            {wxSummary && mapReady && (
              <div className="absolute right-3 top-3 w-44 rounded-lg bg-slate-950/85 px-3 py-2.5 text-[11px] text-slate-200 ring-1 ring-slate-700 backdrop-blur">
                <div className="mb-1.5 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                  <Wind className="h-3 w-3" />
                  {t("fb.weather.summary.title")}
                </div>
                <dl className="space-y-1">
                  {wxSummary.avgWind != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.wind_avg")}</dt>
                      <dd className="font-mono">{wxSummary.avgWind} kt</dd>
                    </div>
                  )}
                  {wxSummary.maxWind != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.wind_max")}</dt>
                      <dd className="font-mono">{wxSummary.maxWind} kt</dd>
                    </div>
                  )}
                  {wxSummary.minTemp != null && wxSummary.maxTemp != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.temp")}</dt>
                      <dd className="font-mono">
                        {wxSummary.minTemp}…{wxSummary.maxTemp}°C
                      </dd>
                    </div>
                  )}
                  {wxSummary.qnh != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.qnh")}</dt>
                      <dd className="font-mono">{wxSummary.qnh} hPa</dd>
                    </div>
                  )}
                  <div className="flex justify-between">
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
            {isLiveFlight && mapReady && (
              <div className="absolute left-3 top-3 max-w-xs rounded-lg bg-slate-950/80 px-3 py-2 text-[11px] text-slate-300 ring-1 ring-slate-700 backdrop-blur">
                {activeLayer === "clouds" && t("fb.weather.note.clouds")}
                {activeLayer === "precip" && t("fb.weather.note.precip")}
              </div>
            )}

            {/* (v4.0.0 P7.9b iter2) Leyenda de la capa de viento —
                explica qué son las líneas para que no se confundan
                con la ruta. */}
            {activeLayer === "wind" && hasWeather && mapReady && (
              <div className="absolute bottom-3 left-3 rounded-lg bg-slate-950/85 px-3 py-2.5 text-[10px] text-slate-300 ring-1 ring-slate-700 backdrop-blur">
                <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                  {t("fb.weather.legend.title")}
                </div>
                <div className="mb-1 flex items-center gap-2">
                  <span className="inline-block h-0.5 w-5 rounded bg-amber-500" />
                  <span>{t("fb.weather.legend.route")}</span>
                </div>
                <div className="mb-1.5 flex items-center gap-2">
                  <span className="text-sky-300">➛</span>
                  <span>{t("fb.weather.legend.wind")}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <span className="h-2 w-2 rounded-sm bg-[#38bdf8]" />
                  <span className="text-slate-500">&lt;20</span>
                  <span className="h-2 w-2 rounded-sm bg-[#22c55e]" />
                  <span className="h-2 w-2 rounded-sm bg-[#facc15]" />
                  <span className="h-2 w-2 rounded-sm bg-[#ef4444]" />
                  <span className="text-slate-500">&gt;50 kt</span>
                </div>
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
          className="absolute bottom-0 right-0 h-4 w-4 cursor-se-resize"
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

type LayerKey =
  | "wind"
  | "clouds"
  | "precip"
  | "tropopause"
  | "sigmet"
  | "turbulence"
  | "icing"
  | "jetstream";

const LAYERS: { key: LayerKey; labelKey: string; Icon: typeof Cloud }[] = [
  { key: "clouds", labelKey: "fb.weather.layer.clouds", Icon: Cloud },
  { key: "precip", labelKey: "fb.weather.layer.precip", Icon: CloudRain },
  { key: "wind", labelKey: "fb.weather.layer.wind", Icon: Wind },
  { key: "tropopause", labelKey: "fb.weather.layer.tropopause", Icon: CloudSnow },
  { key: "sigmet", labelKey: "fb.weather.layer.sigmet", Icon: CloudLightning },
  { key: "turbulence", labelKey: "fb.weather.layer.turbulence", Icon: Zap },
  { key: "icing", labelKey: "fb.weather.layer.icing", Icon: Snowflake },
  { key: "jetstream", labelKey: "fb.weather.layer.jetstream", Icon: Wind },
];
