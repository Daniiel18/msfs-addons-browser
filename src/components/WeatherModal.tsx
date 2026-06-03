import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import maplibregl, { type GeoJSONSource } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import {
  Cloud,
  CloudRain,
  Eye,
  Layers,
  Loader2,
  Snowflake,
  Thermometer,
  Wind,
  X,
} from "lucide-react";
import type { FlightLogEntry, WeatherSample, SimBriefBriefing } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useUnits } from "../lib/units";
import { buildTerminatorPolygon } from "../lib/terminator";
import { segmentTrackCoords } from "../lib/smooth";

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
  const u = useUnits();
  // Visibilidad: métrica en km, imperial en millas terrestres (sm) —
  // el estándar de aviación en EE.UU. (METAR US usa SM).
  const fmtVis = (m: number): string => {
    if (u.system === "metric") {
      return m >= 9999 ? "10+ km" : `${(m / 1000).toFixed(1)} km`;
    }
    return m >= 9999 ? "6+ sm" : `${(m / 1609.34).toFixed(1)} sm`;
  };
  const containerRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  const [mapReady, setMapReady] = useState(false);
  const [activeLayer, setActiveLayer] = useState<LayerKey>("windy");
  const [size, setSize] = useState({ width: 880, height: 560 });
  const resizingRef = useRef<{ startX: number; startY: number; w: number; h: number } | null>(
    null,
  );

  // Vuelo activo si todavía no terminó (sin ended_at).
  const isLiveFlight = entry.endedAt == null;

  // (v4.0.0 P7.9b) Weather samples capturados durante el vuelo.
  const [weather, setWeather] = useState<WeatherSample[] | null>(null);
  const hasWeather = (weather?.length ?? 0) > 0;

  // Carga el track del vuelo, ya SEGMENTADO, para dibujar la ruta.
  const [trackSegments, setTrackSegments] = useState<
    [number, number][][] | null
  >(null);
  useEffect(() => {
    let cancelled = false;
    api
      .getFlightTrack(entry.id)
      .then((pts) => {
        if (cancelled) return;
        // (v3.23.0) Segmentamos en saltos geométricos / gaps temporales
        // → cada tramo es una sub-traza independiente. Evita la recta
        // larga cruzando el mapa que dibujaban los tracks corruptos /
        // duplicados (varias sesiones concatenadas o un glitch de GPS).
        setTrackSegments(
          segmentTrackCoords(
            pts.map((p) => ({ lon: p.lon, lat: p.lat, ts: p.ts })),
          ),
        );
      })
      .catch(() => setTrackSegments([]));
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

  // (v3.32.0 #4) Briefing de SimBrief (METAR/TAF reales de la vida real).
  // On-demand; se muestra SÓLO si el OFP actual matchea origen+destino de
  // este vuelo (si no, no es su clima → no lo mostramos). Falla en
  // silencio si no hay Pilot ID configurado.
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
  // ruta (o el origen si no hay destino). Capas REALES conmutables por el
  // usuario (viento/lluvia/nubes/temp/presión) + timeline. metricWind=kt y
  // metricTemp=°C por coherencia con aviación.
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
      overlay: "wind",
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

  // Inicializa el mapa una sola vez.
  useEffect(() => {
    if (!containerRef.current || mapRef.current) return;

    // (v3.20.0 P7.9e) Fecha para la imagen satelital NASA. Usamos el día
    // del vuelo (UTC). GIBS procesa la pasada del día con ~unas horas de
    // retraso, así que para vuelos de HOY (o futuros por desfase de reloj)
    // caemos a AYER, que siempre está disponible. Para vuelos pasados
    // (el caso normal al revisar) usamos el día exacto.
    const flightDay = (entry.endedAt ?? entry.startedAt ?? "").slice(0, 10);
    const yesterday = new Date(Date.now() - 86_400_000)
      .toISOString()
      .slice(0, 10);
    const gibsDate =
      flightDay && flightDay < yesterday ? flightDay : yesterday;

    const map = new maplibregl.Map({
      container: containerRef.current,
      style: {
        version: 8,
        sources: {
          // (v3.19.0 P7.9d) Basemap SATELITAL (Esri World Imagery —
          // gratis, sin API key) para todo el Weather modal.
          satellite: {
            type: "raster",
            tiles: [
              "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
            ],
            tileSize: 256,
            attribution: "© Esri · Maxar · Earthstar Geographics",
          },
          // (v3.20.0 P7.9e) NASA GIBS — imagen satelital REAL del día del
          // vuelo (VIIRS true-color). Se ven las nubes reales de ese día,
          // como una foto desde el espacio. Gratis, sin API key, histórico.
          // Donde GIBS no tenga dato (hoy sin procesar, noche) se ve el
          // satélite Esri de abajo. Máx zoom nativo ~nivel 8.
          "nasa-gibs": {
            type: "raster",
            tiles: [
              `https://gibs.earthdata.nasa.gov/wmts/epsg3857/best/VIIRS_SNPP_CorrectedReflectance_TrueColor/default/${gibsDate}/GoogleMapsCompatible_Level9/{z}/{y}/{x}.jpg`,
            ],
            tileSize: 256,
            maxzoom: 8,
            attribution: "NASA EOSDIS GIBS",
          },
        },
        layers: [
          {
            id: "basemap",
            type: "raster",
            source: "satellite",
          },
          // Capa de nubes NASA — encima del satélite base, debajo de la
          // ruta y overlays (que se añaden en `load`). Oculta salvo en
          // la pestaña Clouds.
          {
            id: "wx-gibs-layer",
            type: "raster",
            source: "nasa-gibs",
            layout: { visibility: "none" },
            paint: { "raster-opacity": 1 },
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

    // (v3.23.0) Proyección PLANA (mercator, default). Antes usábamos
    // globo, pero al alejar el zoom la Tierra se veía como una bola
    // negra rodeada de espacio — molesto para revisar un vuelo.
    // Mercator llena la vista a cualquier nivel de zoom.

    // Setup de sources/layers UNA vez al primer load.
    map.once("load", () => {
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

      // === Capa raster LIVE de precipitación (RainViewer, sólo en vivo) ===
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

      // (v3.20.0 P7.9e) Las nubes ya NO se dibujan acá: la pestaña Clouds
      // usa la capa raster `wx-gibs-layer` (imagen satelital NASA del día
      // del vuelo), declarada en el style inicial para quedar debajo de
      // la ruta. Acá sólo quedan las cintas derivadas (temp/vis/icing).

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
      map.remove();
      mapRef.current = null;
      setMapReady(false);
    };
  }, [entry.originLat, entry.originLon, entry.destinationLat, entry.destinationLon]);

  // Cuando llegan los puntos del track, los pintamos y ajustamos
  // el viewport al bounding box de la ruta.
  useEffect(() => {
    if (!mapReady || !trackSegments) return;
    const map = mapRef.current;
    if (!map) return;
    try {
      const routeSource = map.getSource("wx-route") as GeoJSONSource | undefined;
      const endpointsSource = map.getSource("wx-endpoints") as
        | GeoJSONSource
        | undefined;
      if (!routeSource || !endpointsSource) return;

      // (v3.23.0) Ruta como MultiLineString — un tramo por segmento.
      // Los saltos/gaps NO se unen con rectas (antes una sola LineString
      // dibujaba la recta larga de los tracks duplicados/corruptos).
      const lines = trackSegments.filter((s) => s.length >= 2);
      routeSource.setData({
        type: "Feature",
        properties: {},
        geometry: { type: "MultiLineString", coordinates: lines },
      });
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

      // Encuadrar a la ruta (todos los puntos de todos los segmentos).
      const bounds = new maplibregl.LngLatBounds();
      const allPts = trackSegments.flat();
      if (allPts.length > 0) {
        allPts.forEach((c) => bounds.extend(c));
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
    trackSegments,
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
    // (v3.20.0 P7.9e) Nubes = imagen satelital NASA del día (raster GIBS).
    vis("wx-gibs-layer", activeLayer === "clouds");
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

  // (v3.19.0 P7.9d) Disponibilidad por capa.
  //   · Derivadas (wind/temp/visibility/icing) → necesitan samples.
  //   · precip → samples (pasado) o RainViewer (live).
  //   · clouds → nubes REALES capturadas (Open-Meteo) por sample.
  const layerAvailable = (key: LayerKey): boolean => {
    switch (key) {
      case "windy":
        // (v3.33.0 #4) Windy funciona para cualquier vuelo con coords
        // (clima en vivo/pronóstico, no depende de samples capturados).
        return true;
      case "wind":
      case "temp":
      case "visibility":
      case "icing":
        return hasWeather;
      case "precip":
        return hasWeather || isLiveFlight;
      case "clouds":
        // (v3.20.0 P7.9e) Imagen satelital NASA del día — funciona para
        // CUALQUIER vuelo (incluso viejos sin weather), porque se basa en
        // la fecha, no en samples capturados.
        return true;
      default:
        return false;
    }
  };

  // El sidebar muestra todas las capas; cada una se habilita según los
  // datos que el vuelo capturó (tooltip explica si falta). Así no hay
  // tabs muertos para vuelos con weather, y los viejos sin datos lo
  // dicen claramente.
  const sidebarLayers = LAYERS;

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
    // FIX: 2 = SIN precipitación; lluvia=4, nieve=8.
    const anyPrecip = weather.some(
      (w) => w.precipState === 4 || w.precipState === 8,
    );
    return { maxWind, avgWind, minTemp, maxTemp, minVis, avgCloud, qnh, anyPrecip };
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

            {/* (v3.33.0 #4) Windy embebido — capa REAL interactiva. Cubre
                el mapa MapLibre (z-20) cuando la pestaña Windy está activa;
                trae su propio selector de capas + timeline. Clima en vivo/
                pronóstico (no el histórico exacto de vuelos viejos). Las
                otras pestañas siguen mostrando los overlays del propio
                vuelo (clima capturado AMBIENT). */}
            {activeLayer === "windy" && windyUrl && (
              <iframe
                title="Windy"
                src={windyUrl}
                className="absolute inset-0 z-20 h-full w-full border-0"
                loading="lazy"
              />
            )}

            {!mapReady && (
              <div className="absolute inset-0 flex items-center justify-center bg-slate-950/60 text-xs text-slate-400">
                <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                {t("fb.weather.loading_map")}
              </div>
            )}
            {/* (v3.25.0) Mientras se reconstruye el clima real del día desde
                Open-Meteo Archive (vuelos sin captura AMBIENT). Evita el
                flash del banner "no disponible" durante la carga. */}
            {!isLiveFlight && weather === null && mapReady && (
              <div className="absolute right-3 top-3 flex max-w-xs items-center gap-2 rounded-lg bg-sky-500/10 px-3 py-2 text-[11px] text-sky-200 ring-1 ring-sky-500/30 backdrop-blur">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("fb.weather.reconstructing")}
              </div>
            )}
            {/* Banner sólo si YA cargó y de verdad no hay datos (sin internet
                para reconstruir desde el archivo real). */}
            {!isLiveFlight && weather !== null && !hasWeather && mapReady && (
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
                  {wxSummary.avgCloud != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.clouds")}</dt>
                      <dd className="font-mono">{wxSummary.avgCloud}%</dd>
                    </div>
                  )}
                  {wxSummary.avgWind != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.wind_avg")}</dt>
                      <dd className="font-mono">{u.fmt.speed(wxSummary.avgWind)}</dd>
                    </div>
                  )}
                  {wxSummary.maxWind != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.wind_max")}</dt>
                      <dd className="font-mono">{u.fmt.speed(wxSummary.maxWind)}</dd>
                    </div>
                  )}
                  {wxSummary.minTemp != null && wxSummary.maxTemp != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.temp")}</dt>
                      <dd className="font-mono">
                        {Math.round(u.conv.temp(wxSummary.minTemp))}…{u.fmt.temp(wxSummary.maxTemp)}
                      </dd>
                    </div>
                  )}
                  {wxSummary.minVis != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.visibility")}</dt>
                      <dd className="font-mono">{fmtVis(wxSummary.minVis)}</dd>
                    </div>
                  )}
                  {wxSummary.qnh != null && (
                    <div className="flex justify-between">
                      <dt className="text-slate-400">{t("fb.weather.summary.qnh")}</dt>
                      <dd className="font-mono">{u.fmt.pressure(wxSummary.qnh)}</dd>
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

            {/* (v3.32.0 #4) Briefing SimBrief — METAR REAL de origen y
                destino del OFP vinculado. Panel abajo-centro (libre de la
                leyenda@izq y el resize@der). Sólo si el OFP matchea. */}
            {briefingWx && mapReady && (
              <div className="absolute bottom-3 left-1/2 z-[1] max-h-[42%] w-[min(30rem,calc(100%-7rem))] -translate-x-1/2 overflow-y-auto rounded-lg bg-slate-950/90 px-3 py-2 text-[10px] text-slate-200 ring-1 ring-sky-700/50 backdrop-blur">
                <div className="mb-1 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-wide text-sky-300">
                  <Cloud className="h-3 w-3" />
                  {t("fb.weather.simbrief.title")}
                </div>
                <dl className="space-y-1.5">
                  {briefingWx.origMetar && (
                    <div>
                      <dt className="font-mono text-[10px] font-semibold text-sky-200">
                        {t("fb.weather.simbrief.dep")} {briefingWx.originIcao ?? ""}
                      </dt>
                      <dd className="whitespace-pre-wrap break-words font-mono leading-relaxed text-slate-200">
                        {briefingWx.origMetar}
                      </dd>
                    </div>
                  )}
                  {briefingWx.destMetar && (
                    <div>
                      <dt className="font-mono text-[10px] font-semibold text-sky-200">
                        {t("fb.weather.simbrief.dest")} {briefingWx.destinationIcao ?? ""}
                      </dt>
                      <dd className="whitespace-pre-wrap break-words font-mono leading-relaxed text-slate-200">
                        {briefingWx.destMetar}
                      </dd>
                    </div>
                  )}
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
                    donde la cinta RECOLOREA la ruta entera. En nubes el
                    manto flota POR ENCIMA, así que la ruta sigue visible. */}
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
  const u = useUnits();
  if (layer === "wind") {
    return (
      <>
        <div className="mb-1.5 flex items-center gap-2">
          <span className="text-sky-300">➛</span>
          <span>{t("fb.weather.legend.wind")}</span>
        </div>
        <GradientBar
          colors={["#38bdf8", "#22c55e", "#facc15", "#ef4444"]}
          left={`<${Math.round(u.conv.speed(20))}`}
          right={`>${Math.round(u.conv.speed(50))} ${u.unit.speed}`}
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
          left={`${Math.round(u.conv.temp(-50))}`}
          right={`${Math.round(u.conv.temp(40))} ${u.unit.temp}`}
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
          right={u.system === "metric" ? "10+ km" : "6+ sm"}
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
  if (layer === "clouds") {
    return (
      <div className="max-w-[12rem] leading-snug text-slate-400">
        {t("fb.weather.legend.clouds_nasa")}
      </div>
    );
  }
  return null;
}

type LayerKey = "windy" | "wind" | "temp" | "precip" | "visibility" | "icing" | "clouds";

const LAYERS: { key: LayerKey; labelKey: string; Icon: typeof Cloud }[] = [
  // (v3.33.0 #4) Windy embebido — mapa interactivo de capas REALES
  // (viento/lluvia/nubes/temp/presión + timeline). Es la vista por
  // defecto; las demás son overlays derivados del clima del propio vuelo.
  { key: "windy", labelKey: "fb.weather.layer.windy", Icon: Layers },
  { key: "clouds", labelKey: "fb.weather.layer.clouds", Icon: Cloud },
  { key: "precip", labelKey: "fb.weather.layer.precip", Icon: CloudRain },
  { key: "wind", labelKey: "fb.weather.layer.wind", Icon: Wind },
  { key: "temp", labelKey: "fb.weather.layer.temp", Icon: Thermometer },
  { key: "visibility", labelKey: "fb.weather.layer.visibility", Icon: Eye },
  { key: "icing", labelKey: "fb.weather.layer.icing", Icon: Snowflake },
];
