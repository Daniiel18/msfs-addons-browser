import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import maplibregl, { type GeoJSONSource } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import {
  Cloud,
  CloudRain,
  Eye,
  Loader2,
  Snowflake,
  Thermometer,
  Wind,
  X,
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
 * (v4.0.0 P7.9b iter3) Construye segmentos LineString consecutivos a lo
 * largo del track, cada uno con una propiedad numérica `val`. Sólo se
 * emite el segmento si AMBOS extremos tienen valor (así no dibujamos
 * "puentes" sobre huecos de datos). El color de cada segmento lo
 * resuelve un `interpolate`/`match` en el paint del layer.
 *
 * Usamos el valor del extremo inicial (`a`) en vez del promedio para
 * evitar valores fraccionarios en propiedades categóricas (precip:
 * 1=lluvia, 2=nieve → el promedio daría 1.5 y rompería el `match`).
 */
function buildValueSegments(
  samples: WeatherSample[],
  getVal: (w: WeatherSample) => number | null,
): GeoJSON.FeatureCollection {
  const feats: GeoJSON.Feature[] = [];
  for (let i = 0; i < samples.length - 1; i++) {
    const a = samples[i];
    const b = samples[i + 1];
    if (
      !Number.isFinite(a.lat) ||
      !Number.isFinite(a.lon) ||
      !Number.isFinite(b.lat) ||
      !Number.isFinite(b.lon)
    )
      continue;
    const va = getVal(a);
    const vb = getVal(b);
    if (va == null || vb == null) continue;
    feats.push({
      type: "Feature",
      properties: { val: va },
      geometry: {
        type: "LineString",
        coordinates: [
          [a.lon, a.lat],
          [b.lon, b.lat],
        ],
      },
    });
  }
  return { type: "FeatureCollection", features: feats };
}

/** Precipitación → código categórico: 4 (SimConnect rain) → 1,
 *  8 (snow) → 2, 2 (none) / null → sin segmento. */
function precipCode(w: WeatherSample): number | null {
  if (w.precipState === 4) return 1; // rain
  if (w.precipState === 8) return 2; // snow
  return null;
}

/** Riesgo de engelamiento derivado de OAT + humedad visible.
 *  Ventana de agua sobreenfriada ≈ +2°C … −20°C. Severidad 1..3.
 *  Devuelve `null` cuando no hay riesgo (no se dibuja segmento). */
function icingSeverity(w: WeatherSample): number | null {
  const oat = w.oatC;
  if (oat == null || oat > 2 || oat < -20) return null;
  const wet =
    w.precipState === 4 ||
    w.precipState === 8 ||
    (w.visibilityM != null && w.visibilityM < 5000);
  if (!wet) return null;
  // Pico de riesgo en la banda −2…−10°C; lluvia engelante = severo.
  if (oat <= 0 && oat >= -10) return w.precipState === 4 ? 3 : 2;
  return 1;
}

// === Escalas de color (compartidas entre el paint del mapa y la leyenda) ===
const TEMP_RAMP: (number | string)[] = [
  -50, "#4338ca",
  -30, "#3b82f6",
  -15, "#38bdf8",
  0, "#5eead4",
  10, "#22c55e",
  25, "#f59e0b",
  40, "#ef4444",
];
const VIS_RAMP: (number | string)[] = [
  0, "#ef4444",
  1500, "#f97316",
  3000, "#facc15",
  5000, "#84cc16",
  10000, "#22c55e",
  20000, "#38bdf8",
];
const WIND_RAMP: (number | string)[] = [
  5, "#38bdf8",
  20, "#22c55e",
  35, "#facc15",
  50, "#ef4444",
];

/**
 * (v4.0.0 — P7 iter 3) Weather modal del FlightBook.
 *
 * Modal floating resizable inspirado en Windy. Muestra la ruta del
 * vuelo seleccionado sobre un mapa MapLibre con globo 3D y capas
 * meteorológicas.
 *
 * ## Estrategia de datos
 *
 * Durante el vuelo el watcher captura el clima AMBIENT del sim por
 * cada track sample (`flight_log_track`): viento (dir+vel), OAT,
 * presión, visibilidad y estado de precipitación. Eso permite
 * reconstruir el clima REAL del vuelo sin depender de APIs de clima
 * histórico (que son de pago).
 *
 * ### Capas derivadas de los samples (funcionan en cualquier vuelo
 * que haya capturado weather — live o pasado):
 *
 *   · **Wind** — barbas que apuntan a favor del viento, color por kt.
 *   · **Temperature** — cinta coloreada por OAT (frío→cálido).
 *   · **Precipitation** — segmentos donde el sim reportó lluvia/nieve
 *     (`AMBIENT PRECIP STATE`: 4=rain, 8=snow; 2=none).
 *   · **Visibility** — cinta coloreada por visibilidad (roja=baja).
 *   · **Icing** — corredores de riesgo derivados de OAT + humedad.
 *
 * ### Capas LIVE (sólo vuelo en curso, tiles RainViewer sin API key):
 *
 *   · **Clouds** — satélite.
 *   · **Precipitation** — radar (sustituye a la derivada cuando el
 *     vuelo está activo, porque el radar es más rico).
 *
 * Las capas de aviación que requieren feeds online sin archivo
 * histórico gratuito (SIGMET, Jet Stream, Tropopausa, Turbulencia) se
 * quitaron del modal: no se pueden reconstruir para un vuelo pasado.
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
  const [activeLayer, setActiveLayer] = useState<LayerKey>("wind");
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

      // Ruta del vuelo (halo blanco + línea ámbar — distinta de las
      // capas weather que usan tonos fríos/calientes por dato).
      map.addSource("wx-route", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-route-halo",
        type: "line",
        source: "wx-route",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "rgba(255, 255, 255, 0.55)",
          "line-width": 4,
        },
      });
      map.addLayer({
        id: "wx-route-line",
        type: "line",
        source: "wx-route",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#f59e0b",
          "line-width": 2.5,
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

      // === Capas raster LIVE (RainViewer) ===
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
        paint: { "raster-opacity": 0.7 },
        layout: { visibility: "none" },
      });

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
        paint: { "raster-opacity": 0.65 },
        layout: { visibility: "none" },
      });

      // === Capas derivadas de los weather samples (route overlays) ===

      // Temperatura (OAT) — cinta coloreada frío→cálido.
      map.addSource("wx-temp", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-temp-line",
        type: "line",
        source: "wx-temp",
        layout: { "line-cap": "round", "line-join": "round", visibility: "none" },
        paint: {
          "line-width": 4,
          "line-color": ["interpolate", ["linear"], ["get", "val"], ...TEMP_RAMP] as never,
        },
      });

      // Visibilidad — cinta coloreada (roja=baja, azul=excelente).
      map.addSource("wx-vis", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-vis-line",
        type: "line",
        source: "wx-vis",
        layout: { "line-cap": "round", "line-join": "round", visibility: "none" },
        paint: {
          "line-width": 4,
          "line-color": ["interpolate", ["linear"], ["get", "val"], ...VIS_RAMP] as never,
        },
      });

      // Engelamiento — corredores de riesgo (glow violeta + línea).
      map.addSource("wx-icing", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-icing-glow",
        type: "line",
        source: "wx-icing",
        layout: { "line-cap": "round", "line-join": "round", visibility: "none" },
        paint: {
          "line-width": 10,
          "line-blur": 4,
          "line-opacity": 0.45,
          "line-color": "#818cf8",
        },
      });
      map.addLayer({
        id: "wx-icing-line",
        type: "line",
        source: "wx-icing",
        layout: { "line-cap": "round", "line-join": "round", visibility: "none" },
        paint: {
          "line-width": 4.5,
          "line-color": [
            "interpolate",
            ["linear"],
            ["get", "val"],
            1, "#a5b4fc",
            2, "#818cf8",
            3, "#6d28d9",
          ],
        },
      });

      // Precipitación derivada (pasado) — segmentos lluvia/nieve.
      map.addSource("wx-precip-pts", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-precip-pts-line",
        type: "line",
        source: "wx-precip-pts",
        layout: { "line-cap": "round", "line-join": "round", visibility: "none" },
        paint: {
          "line-width": 5,
          "line-color": [
            "match",
            ["get", "val"],
            1, "#3b82f6", // lluvia — azul
            2, "#bae6fd", // nieve — celeste pálido
            "#3b82f6",
          ],
        },
      });

      // Viento — barbas (línea) + cabeza (circle) coloreadas por kt.
      map.addSource("wx-wind", { type: "geojson", data: empty });
      map.addLayer({
        id: "wx-wind-line",
        type: "line",
        source: "wx-wind",
        layout: { "line-cap": "round", visibility: "none" },
        paint: {
          "line-width": 1.6,
          "line-color": ["interpolate", ["linear"], ["get", "speed"], ...WIND_RAMP] as never,
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
          "circle-color": ["interpolate", ["linear"], ["get", "speed"], ...WIND_RAMP] as never,
        },
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

  // (v4.0.0 P7.9b iter3) Cuando llegan los weather samples, generamos
  // TODAS las capas derivadas (viento, temp, visibilidad, icing,
  // precip) y las cargamos en sus sources.
  useEffect(() => {
    if (!mapReady || !weather) return;
    const map = mapRef.current;
    if (!map) return;

    // --- Viento: barbas (sampleadas a ~40 para no saturar) ---
    const windSrc = map.getSource("wx-wind") as GeoJSONSource | undefined;
    if (windSrc) {
      const feats: GeoJSON.Feature[] = [];
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
      windSrc.setData({ type: "FeatureCollection", features: feats });
    }

    // --- Capas de cinta (segmentos por dato) ---
    const setSeg = (
      id: string,
      getVal: (w: WeatherSample) => number | null,
    ) => {
      const src = map.getSource(id) as GeoJSONSource | undefined;
      if (src) src.setData(buildValueSegments(weather, getVal));
    };
    setSeg("wx-temp", (w) => w.oatC);
    setSeg("wx-vis", (w) =>
      w.visibilityM != null ? Math.min(w.visibilityM, 20000) : null,
    );
    setSeg("wx-icing", icingSeverity);
    setSeg("wx-precip-pts", precipCode);
  }, [weather, mapReady]);

  // Cambia qué capa está visible según el tab activo.
  useEffect(() => {
    if (!mapReady) return;
    const map = mapRef.current;
    if (!map) return;
    const vis = (id: string, on: boolean) => {
      try {
        map.setLayoutProperty(id, "visibility", on ? "visible" : "none");
      } catch {
        /* layer puede no existir aún */
      }
    };
    vis("wx-clouds-layer", activeLayer === "clouds");
    // Precip: radar RainViewer en vivo, segmentos derivados en pasado.
    vis("wx-precip-layer", activeLayer === "precip" && isLiveFlight);
    vis("wx-precip-pts-line", activeLayer === "precip" && !isLiveFlight);
    vis("wx-temp-line", activeLayer === "temp");
    vis("wx-vis-line", activeLayer === "visibility");
    vis("wx-icing-glow", activeLayer === "icing");
    vis("wx-icing-line", activeLayer === "icing");
    const windOn = activeLayer === "wind";
    vis("wx-wind-line", windOn);
    vis("wx-wind-head", windOn);
  }, [activeLayer, mapReady, isLiveFlight]);

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

  // Cuando el modal cambia de tamaño, re-disparamos resize del mapa.
  useEffect(() => {
    if (!mapReady) return;
    const map = mapRef.current;
    if (!map) return;
    map.resize();
  }, [size.width, size.height, mapReady]);

  // (v4.0.0 P7.9b iter3) Disponibilidad por capa.
  //   · Derivadas (wind/temp/visibility/icing) → necesitan samples.
  //   · precip → samples (pasado) o RainViewer (live).
  //   · clouds → sólo live (RainViewer no tiene histórico).
  const layerAvailable = (key: LayerKey): boolean => {
    switch (key) {
      case "wind":
      case "temp":
      case "visibility":
      case "icing":
        return hasWeather;
      case "precip":
        return hasWeather || isLiveFlight;
      case "clouds":
        return isLiveFlight;
      default:
        return false;
    }
  };

  // El sidebar muestra las 5 capas derivadas siempre (deshabilitadas
  // con tooltip si el vuelo no capturó weather), y Clouds sólo cuando
  // el vuelo está en vivo (RainViewer). Así no hay tabs muertos.
  const sidebarLayers = LAYERS.filter(
    (l) => l.key !== "clouds" || isLiveFlight,
  );

  // Si la capa activa no está disponible, saltamos a la primera que sí.
  useEffect(() => {
    if (layerAvailable(activeLayer)) return;
    const first = sidebarLayers.find((l) => layerAvailable(l.key));
    if (first) setActiveLayer(first.key);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasWeather, isLiveFlight, activeLayer]);

  // (v4.0.0 P7.9b) Resumen de weather para el panel lateral.
  const wxSummary = (() => {
    if (!hasWeather || !weather) return null;
    const winds = weather.map((w) => w.windSpeedKt).filter((v): v is number => v != null);
    const temps = weather.map((w) => w.oatC).filter((v): v is number => v != null);
    const baros = weather.map((w) => w.baroHpa).filter((v): v is number => v != null);
    const viss = weather.map((w) => w.visibilityM).filter((v): v is number => v != null);
    const maxWind = winds.length ? Math.max(...winds) : null;
    const avgWind = winds.length
      ? Math.round(winds.reduce((a, b) => a + b, 0) / winds.length)
      : null;
    const minTemp = temps.length ? Math.round(Math.min(...temps)) : null;
    const maxTemp = temps.length ? Math.round(Math.max(...temps)) : null;
    const minVis = viss.length ? Math.round(Math.min(...viss)) : null;
    const qnh = baros.length ? baros[0] : null;
    // FIX: 2 = SIN precipitación; lluvia=4, nieve=8.
    const anyPrecip = weather.some(
      (w) => w.precipState === 4 || w.precipState === 8,
    );
    return { maxWind, avgWind, minTemp, maxTemp, minVis, qnh, anyPrecip };
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
            {sidebarLayers.map((l) => {
              const available = layerAvailable(l.key);
              return (
                <button
                  key={l.key}
                  onClick={() => available && setActiveLayer(l.key)}
                  disabled={!available}
                  title={available ? undefined : t("fb.weather.no_weather_captured")}
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
            {/* Banner "histórico no disponible" SOLO si el vuelo pasado
                NO capturó weather (pre-v3.16.0 / VAS). */}
            {!isLiveFlight && !hasWeather && mapReady && (
              <div className="absolute right-3 top-3 max-w-xs rounded-lg bg-amber-500/10 px-3 py-2 text-[11px] text-amber-200 ring-1 ring-amber-500/30 backdrop-blur">
                {t("fb.weather.historical_unavailable")}
              </div>
            )}

            {/* Panel resumen de weather del vuelo. */}
            {wxSummary && mapReady && (
              <div className="absolute right-3 top-3 w-44 rounded-lg bg-slate-950/85 px-3 py-2.5 text-[11px] text-slate-200 ring-1 ring-slate-700 backdrop-blur">
                <div className="mb-1.5 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                  <Cloud className="h-3 w-3" />
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
                  {wxSummary.minVis != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.visibility")}</dt>
                      <dd className="font-mono">
                        {wxSummary.minVis >= 9999
                          ? "10+ km"
                          : `${(wxSummary.minVis / 1000).toFixed(1)} km`}
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

            {/* Nota contextual por capa (arriba-izquierda). */}
            {mapReady && (
              <div className="absolute left-3 top-3 max-w-[15rem] rounded-lg bg-slate-950/80 px-3 py-2 text-[11px] text-slate-300 ring-1 ring-slate-700 backdrop-blur">
                {t(`fb.weather.note.${activeLayer}`)}
              </div>
            )}

            {/* Leyenda dinámica por capa (abajo-izquierda). */}
            {mapReady && layerAvailable(activeLayer) && (
              <div className="absolute bottom-3 left-3 rounded-lg bg-slate-950/85 px-3 py-2.5 text-[10px] text-slate-300 ring-1 ring-slate-700 backdrop-blur">
                <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                  {t("fb.weather.legend.title")}
                </div>
                {/* La ruta queda visible (ámbar) salvo en temp/visibilidad,
                    donde la cinta de color RECOLOREA la ruta entera. */}
                {activeLayer !== "temp" && activeLayer !== "visibility" && (
                  <div className="mb-1.5 flex items-center gap-2">
                    <span className="inline-block h-0.5 w-5 rounded bg-amber-500" />
                    <span>{t("fb.weather.legend.route")}</span>
                  </div>
                )}
                <LayerLegend layer={activeLayer} />
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

/** Barra de gradiente reutilizable para las leyendas de cinta. */
function GradientBar({
  colors,
  left,
  right,
}: {
  colors: string[];
  left: string;
  right: string;
}) {
  return (
    <div className="flex items-center gap-1.5">
      <span className="text-slate-500">{left}</span>
      <span
        className="h-2 w-20 rounded-sm"
        style={{
          background: `linear-gradient(to right, ${colors.join(", ")})`,
        }}
      />
      <span className="text-slate-500">{right}</span>
    </div>
  );
}

/** Leyenda específica de la capa activa. */
function LayerLegend({ layer }: { layer: LayerKey }) {
  if (layer === "wind") {
    return (
      <>
        <div className="mb-1.5 flex items-center gap-2">
          <span className="text-sky-300">➛</span>
          <span>{t("fb.weather.legend.wind")}</span>
        </div>
        <GradientBar
          colors={["#38bdf8", "#22c55e", "#facc15", "#ef4444"]}
          left="<20"
          right=">50 kt"
        />
      </>
    );
  }
  if (layer === "temp") {
    return (
      <>
        <div className="mb-1.5">{t("fb.weather.legend.temp")}</div>
        <GradientBar
          colors={["#4338ca", "#38bdf8", "#5eead4", "#22c55e", "#f59e0b", "#ef4444"]}
          left="-50"
          right="+40 °C"
        />
      </>
    );
  }
  if (layer === "visibility") {
    return (
      <>
        <div className="mb-1.5">{t("fb.weather.legend.visibility")}</div>
        <GradientBar
          colors={["#ef4444", "#f97316", "#facc15", "#22c55e", "#38bdf8"]}
          left={`0 (${t("fb.weather.legend.low")})`}
          right="10+ km"
        />
      </>
    );
  }
  if (layer === "icing") {
    return (
      <>
        <div className="mb-1.5">{t("fb.weather.legend.icing")}</div>
        <div className="flex items-center gap-1.5">
          <span className="h-2 w-2 rounded-sm bg-[#a5b4fc]" />
          <span className="text-slate-500">{t("fb.weather.legend.light")}</span>
          <span className="h-2 w-2 rounded-sm bg-[#818cf8]" />
          <span className="h-2 w-2 rounded-sm bg-[#6d28d9]" />
          <span className="text-slate-500">{t("fb.weather.legend.severe")}</span>
        </div>
      </>
    );
  }
  if (layer === "precip") {
    return (
      <>
        <div className="mb-1.5">{t("fb.weather.legend.precip")}</div>
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5">
            <span className="h-2 w-4 rounded-sm bg-[#3b82f6]" />
            <span>{t("fb.weather.legend.rain")}</span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="h-2 w-4 rounded-sm bg-[#bae6fd]" />
            <span>{t("fb.weather.legend.snow")}</span>
          </div>
        </div>
      </>
    );
  }
  // clouds — RainViewer, sin escala propia.
  return null;
}

type LayerKey = "wind" | "temp" | "precip" | "visibility" | "icing" | "clouds";

const LAYERS: { key: LayerKey; labelKey: string; Icon: typeof Cloud }[] = [
  { key: "wind", labelKey: "fb.weather.layer.wind", Icon: Wind },
  { key: "temp", labelKey: "fb.weather.layer.temp", Icon: Thermometer },
  { key: "precip", labelKey: "fb.weather.layer.precip", Icon: CloudRain },
  { key: "visibility", labelKey: "fb.weather.layer.visibility", Icon: Eye },
  { key: "icing", labelKey: "fb.weather.layer.icing", Icon: Snowflake },
  { key: "clouds", labelKey: "fb.weather.layer.clouds", Icon: Cloud },
];
