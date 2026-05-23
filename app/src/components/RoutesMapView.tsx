import { useEffect, useMemo, useRef, useState } from "react";
import maplibregl, { type GeoJSONSource } from "maplibre-gl";

const tracingLog = (msg: string) =>
  console.info(`[RoutesMapView] ${msg}`);
import "maplibre-gl/dist/maplibre-gl.css";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { greatCircleLine } from "../lib/greatCircle";
import {
  smoothCatmullRom,
  sanitizeTrackCoordsWithTs,
} from "../lib/smooth";
import { api } from "../lib/tauri";
import type { FlightTrackPoint } from "../lib/types";

/** (v2.0.0) Icono del avión — silueta TOP-VIEW (vista cenital) tal
 *  como se ve un avión desde arriba en mapas de aviación. La punta
 *  está en y=2 (norte, heading 0°) y el viewBox está centrado en
 *  (12, 12), de modo que MapLibre `setRotation(heading)` lo rota
 *  exactamente alrededor del centroide.
 *
 *  Path tomado del set Material Icons (`flight` / `airplanemode_active`),
 *  un icono estándar de la industria — no inventado. */
const PLANE_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="36" height="36"><path fill="#fbbf24" stroke="#0f172a" stroke-width="0.7" stroke-linejoin="round" d="M21 16v-2l-8-5V3.5C13 2.67 12.33 2 11.5 2S10 2.67 10 3.5V9l-8 5v2l8-2.5V19l-2 1.5V22l3.5-1 3.5 1v-1.5L13 19v-5.5z"/></svg>`;

/**
 * Mapa **sólo de rutas** — vive dentro del FlightBook.
 *
 * A diferencia de `MapView` (que pinta los aeropuertos del catálogo
 * con clusters), este mapa muestra exclusivamente:
 *   · Líneas SimBrief (planes de vuelo) — color cyan/azul.
 *   · Líneas SimConnect (vuelos reales del flight_log) — color verde
 *     esmeralda con halo blanco para destacar.
 *   · Marcadores compactos en origen y destino de cada vuelo.
 *
 * Diseño:
 *   · Estilo basemap **oscuro** (CARTO Dark Matter) — encaja con el
 *     theme de la app. El mapa de scenery usa el claro de OSM.
 *   · Auto-fitBounds al montar para que TODAS las rutas quepan en
 *     el viewport sin que el usuario tenga que hacer pan/zoom.
 *   · Sin clusters, sin popups complejos — la lista de vuelos vive
 *     en la tabla del FlightBook; el mapa es una visualización.
 */
export function RoutesMapView({
  className,
  height = 360,
  selectedFlightId,
}: {
  className?: string;
  /** Altura del canvas. Acepta número (px) o string CSS (`"60vh"`,
   *  `"calc(100vh - 11rem)"`). Default 360 — encaja arriba de
   *  la tabla del FlightBook sin robarle protagonismo. */
  height?: number | string;
  /** Cuando viene set, el mapa cambia al modo "detalle de un vuelo":
   *  oculta el resto, pinta la polyline real (track points cada 10s)
   *  de ese vuelo y hace fitBounds sobre ella. Cuando es `null` o
   *  `undefined`, vuelve al modo "globo con todos los vuelos". */
  selectedFlightId?: number | null;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  const didAutoFitRef = useRef(false);
  // (v1.1.4) Marker DOM del avión en vivo. Lo mantenemos en ref para
  // poder hacer setLngLat + rotación sin reconstruir el elemento cada
  // tick — el rerender de React no toca el mapa, sólo este efecto.
  const planeMarkerRef = useRef<maplibregl.Marker | null>(null);
  const planeElementRef = useRef<HTMLDivElement | null>(null);
  // (v1.1.3) Flag para saber si los sources/layers ya están
  // inicializados en el mapa. Resuelve el bug intermitente del
  // redraw: antes la lógica de "addSource si no existe" podía
  // dispararse en múltiples efectos paralelos durante el load
  // inicial, dejando el state inconsistente. Ahora init es ONCE
  // en el on("load"), y todos los updates son `setData` puros.
  const [mapReady, setMapReady] = useState(false);

  const simbriefFlights = useSimBriefStore((s) => s.flights);
  const flightLogEntries = useFlightLogStore((s) => s.entries);
  // (v1.1.4) Estado en vivo del watcher SimConnect — usado para
  // pintar el avión en el mapa cuando hay vuelo en curso.
  const flightStatus = useFlightLogStore((s) => s.status);
  // Cache local de la traza real del vuelo seleccionado — la
  // pedimos al backend con `getFlightTrack` y la convertimos en
  // GeoJSON LineString. Cuando `selectedFlightId` es null/undefined,
  // este state queda vacío y el mapa vuelve a las great-circles.
  const [trackPoints, setTrackPoints] = useState<FlightTrackPoint[]>([]);
  const [loadingTrack, setLoadingTrack] = useState(false);
  const showSimbriefLines = useSettingsStore(
    (s) => s.settings.showSimbriefLines,
  );
  const showSimconnectLines = useSettingsStore(
    (s) => s.settings.showSimconnectLines,
  );

  // Inicialización del mapa — globo terráqueo SATELITAL.
  // Usamos un style raster mínimo con tiles de ESRI World Imagery
  // (free, attribution requerida). Combinado con `setProjection(
  // 'globe')` queda un globo 3D con texturas reales — bastante más
  // bonito que el vector dark vacío.
  useEffect(() => {
    if (mapRef.current || !containerRef.current) return;

    const satelliteStyle: maplibregl.StyleSpecification = {
      version: 8,
      sources: {
        "esri-imagery": {
          type: "raster",
          tiles: [
            "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}",
          ],
          tileSize: 256,
          attribution:
            "Tiles © Esri — World Imagery (Esri, Maxar, Earthstar Geographics)",
          maxzoom: 19,
        },
        // Labels semi-transparentes encima para legibilidad —
        // tomados del basemap de CARTO sin background, solo
        // etiquetas blancas.
        "carto-labels": {
          type: "raster",
          tiles: [
            "https://cartodb-basemaps-a.global.ssl.fastly.net/dark_only_labels/{z}/{x}/{y}.png",
            "https://cartodb-basemaps-b.global.ssl.fastly.net/dark_only_labels/{z}/{x}/{y}.png",
            "https://cartodb-basemaps-c.global.ssl.fastly.net/dark_only_labels/{z}/{x}/{y}.png",
          ],
          tileSize: 256,
          attribution: "Labels © CARTO · © OpenStreetMap contributors",
        },
      },
      layers: [
        {
          id: "esri-imagery-layer",
          type: "raster",
          source: "esri-imagery",
        },
        {
          id: "carto-labels-layer",
          type: "raster",
          source: "carto-labels",
          paint: { "raster-opacity": 0.8 },
        },
      ],
    };

    const map = new maplibregl.Map({
      container: containerRef.current,
      style: satelliteStyle,
      center: [0, 25],
      zoom: 1.2,
      attributionControl: false,
    });
    map.addControl(new maplibregl.NavigationControl({ showCompass: true }), "top-right");
    map.addControl(new maplibregl.AttributionControl({ compact: true }), "bottom-right");

    // Aplicar projection globe en cada style.load. MapLibre 5 lo
    // soporta nativo pero `setProjection` debe llamarse **después**
    // de que el estilo cargó — pasarlo en el constructor no está
    // en MapOptions todavía.
    const ensureGlobe = () => {
      try {
        map.setProjection({ type: "globe" });
        tracingLog("globe projection aplicada OK");
      } catch (e) {
        console.warn("globe projection falló:", e);
      }
    };

    // (v1.1.3) Inicializa TODOS los sources + layers ONCE al cargar
    // el estilo. Después de esto, los effects sólo hacen setData —
    // sin race conditions del patrón "addSource si no existe".
    const empty: GeoJSON.FeatureCollection = {
      type: "FeatureCollection",
      features: [],
    };
    const initSources = () => {
      // SimBrief — cyan tenue, glow + line dashed.
      map.addSource("rt-simbrief", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-simbrief-line-glow",
        type: "line",
        source: "rt-simbrief",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": "#0ea5e9", "line-width": 3.5, "line-opacity": 0.25 },
      });
      map.addLayer({
        id: "rt-simbrief-line",
        type: "line",
        source: "rt-simbrief",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#7dd3fc",
          "line-width": 1.8,
          "line-dasharray": [2, 2],
        },
      });
      // FlightLog — verde esmeralda con halo (vuelos reales).
      map.addSource("rt-flightlog", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-flightlog-line-glow",
        type: "line",
        source: "rt-flightlog",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": "#34d399", "line-width": 5, "line-opacity": 0.35 },
      });
      map.addLayer({
        id: "rt-flightlog-line",
        type: "line",
        source: "rt-flightlog",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": "#34d399", "line-width": 2.4 },
      });
      // Endpoints — círculos en origen/destino.
      map.addSource("rt-endpoints", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-endpoints-circle",
        type: "circle",
        source: "rt-endpoints",
        paint: {
          "circle-radius": 4,
          "circle-color": [
            "case",
            ["==", ["get", "kind"], "real"],
            "#34d399",
            "#7dd3fc",
          ],
          "circle-stroke-width": 1.5,
          "circle-stroke-color": "#0f172a",
          "circle-opacity": 0.95,
        },
      });
      // Track real — ámbar (modo detalle). Encima de todo para que
      // se vea bien sobre el globo satelital.
      map.addSource("rt-track", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-track-line-glow",
        type: "line",
        source: "rt-track",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": "#fbbf24", "line-width": 6, "line-opacity": 0.35 },
      });
      map.addLayer({
        id: "rt-track-line",
        type: "line",
        source: "rt-track",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: { "line-color": "#fbbf24", "line-width": 2.8 },
      });
      // (v1.1.4) Live track — naranja/rojo dasheado para el vuelo
      // EN CURSO. Distinto color del flightlog (verde) y track real
      // de un vuelo cerrado (ámbar). El usuario lo pidió:
      // "implementemos que cuando haya un vuelo en curso se muestre
      // la linea diferente y con el avion marcando por donde va".
      map.addSource("rt-live", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-live-line-glow",
        type: "line",
        source: "rt-live",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#f97316",
          "line-width": 6,
          "line-opacity": 0.35,
        },
      });
      map.addLayer({
        id: "rt-live-line",
        type: "line",
        source: "rt-live",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#fb923c",
          "line-width": 2.6,
          "line-dasharray": [2, 1.5],
        },
      });
      // (v3.1.0) Línea proyectada — great-circle desde la posición
      // actual del avión hasta el destino del OFP. Color cyan apagado
      // dashed para diferenciarla del track real (ámbar) y de los
      // planes SimBrief históricos. Aparece sólo cuando hay vuelo en
      // curso + destination conocido + posición SimConnect.
      map.addSource("rt-projection", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-projection-line",
        type: "line",
        source: "rt-projection",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#67e8f9",
          "line-width": 1.8,
          "line-opacity": 0.75,
          "line-dasharray": [3, 2],
        },
      });
      setMapReady(true);
    };

    if (map.isStyleLoaded()) {
      ensureGlobe();
      initSources();
    }
    map.on("style.load", ensureGlobe);
    // Sólo init sources la PRIMERA vez. Style reloads (raros, p.ej.
    // toggle de tema) no deberían reinitializar todo — además los
    // sources sobreviven al style swap en maplibre.
    map.once("load", initSources);

    mapRef.current = map;
    return () => {
      map.off("style.load", ensureGlobe);
      map.remove();
      mapRef.current = null;
      setMapReady(false);
    };
  }, []);

  // Cuando cambia el vuelo seleccionado, fetcheamos su track real.
  // (v1.1.2) Limpiamos trackPoints INMEDIATAMENTE para evitar mostrar
  // la traza del vuelo anterior durante el fetch — bug que reportó el
  // usuario: click en FMMI→FIMP, pero el mapa seguía mostrando
  // HKJK→FMMI hasta hacer un segundo click.
  useEffect(() => {
    let cancelled = false;
    setTrackPoints([]);
    if (selectedFlightId == null) {
      setLoadingTrack(false);
      return;
    }
    setLoadingTrack(true);
    api
      .getFlightTrack(selectedFlightId)
      .then((pts) => {
        if (!cancelled) setTrackPoints(pts);
      })
      .catch((e) => {
        console.warn("getFlightTrack falló:", e);
        if (!cancelled) setTrackPoints([]);
      })
      .finally(() => {
        if (!cancelled) setLoadingTrack(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedFlightId]);

  // (v1.1.4) Track del VUELO EN CURSO (endedAt = null). Cargado
  // del backend cada vez que cambia el flight id activo. Mientras
  // hay un vuelo abierto, polling cada 10s en sync con el
  // sample del backend (que escribe un punto cada 10s).
  const inFlightEntry = useMemo(
    () => flightLogEntries.find((e) => e.endedAt === null),
    [flightLogEntries],
  );
  const inFlightId = inFlightEntry?.id ?? null;
  const [liveTrackPoints, setLiveTrackPoints] = useState<FlightTrackPoint[]>([]);

  useEffect(() => {
    if (inFlightId == null) {
      setLiveTrackPoints([]);
      return;
    }
    let cancelled = false;
    const tick = () => {
      api
        .getFlightTrack(inFlightId)
        .then((pts) => {
          if (!cancelled) setLiveTrackPoints(pts);
        })
        .catch((e) => console.warn("getFlightTrack (live) falló:", e));
    };
    tick();
    const t = setInterval(tick, 10_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [inFlightId]);

  // GeoJSON del live track — añadimos la posición actual de
  // SimConnect (más fresca que el polling 10s) como último punto.
  const liveTrackGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.LineString>
  >(() => {
    if (inFlightId == null) {
      return { type: "FeatureCollection", features: [] };
    }
    // (v3.4.0) Llevamos timestamp al sanitizador para detectar gaps
    // de tiempo reales (sim pausado, reconexión) además de los saltos
    // geométricos. Esto resuelve la "línea recta fea" que el usuario
    // reportó cuando el flight log tenía un segmento posterior a una
    // pausa de varios minutos.
    const rawPoints = liveTrackPoints.map((p) => ({
      lon: p.lon,
      lat: p.lat,
      ts: p.ts,
    }));
    // Append posición live si el watcher reporta coords. Sin ts
    // exacto, pero el sanitizador lo trata como "siempre cercano al
    // anterior" — sólo aplica los thresholds geométricos.
    if (
      flightStatus?.simconnectConnected &&
      flightStatus.currentLat != null &&
      flightStatus.currentLon != null
    ) {
      rawPoints.push({
        lon: flightStatus.currentLon,
        lat: flightStatus.currentLat,
        ts: new Date().toISOString(),
      });
    }
    const sanitized = sanitizeTrackCoordsWithTs(rawPoints);
    const coords: [number, number][] = sanitized.map(
      (p) => [p.lon, p.lat] as [number, number],
    );
    if (coords.length < 2) {
      return { type: "FeatureCollection", features: [] };
    }
    // (v2.2.0) Suavizado Catmull-Rom — el sample crudo cada 10s da
    // un trazo poligonal en giros (approaches, holds). Pasamos por
    // la spline antes de mandarlo a MapLibre.
    const smooth = smoothCatmullRom(coords, 8);
    return {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: { flightId: inFlightId, live: true },
          geometry: { type: "LineString", coordinates: smooth },
        },
      ],
    };
  }, [inFlightId, liveTrackPoints, flightStatus]);

  // (v3.1.0) Geometría de la línea PROYECTADA — great-circle desde
  // la posición actual del avión hasta el destino del vuelo activo.
  // Sólo se renderiza cuando:
  //   · SimConnect está vivo + currentLat/Lon conocidos.
  //   · Hay un vuelo flight_log abierto (status.originIcao desde el
  //     watcher) Y existe un OFP de SimBrief con ese origen + dest.
  //
  // Sin OFP no hay destinationLat/Lon → no se dibuja.
  const projectionGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.MultiLineString>
  >(() => {
    if (!flightStatus?.simconnectConnected) {
      return { type: "FeatureCollection", features: [] };
    }
    const curLat = flightStatus.currentLat;
    const curLon = flightStatus.currentLon;
    if (curLat == null || curLon == null) {
      return { type: "FeatureCollection", features: [] };
    }
    // Match SimBrief OFP por originIcao del watcher (que viene del
    // simbrief_flights latest_recent_simbrief).
    const origin = flightStatus.originIcao;
    if (!origin) return { type: "FeatureCollection", features: [] };
    const matchedOfp = simbriefFlights.find(
      (f) => f.originIcao === origin,
    );
    if (!matchedOfp) {
      return { type: "FeatureCollection", features: [] };
    }
    // greatCircleLine devuelve [][]  — múltiples polylines cuando la
    // ruta cruza la dateline. Usamos MultiLineString para renderizar
    // todos los segmentos.
    const segments = greatCircleLine(
      curLon,
      curLat,
      matchedOfp.destinationLon,
      matchedOfp.destinationLat,
    );
    if (segments.length === 0) {
      return { type: "FeatureCollection", features: [] };
    }
    return {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: { projection: true },
          geometry: { type: "MultiLineString", coordinates: segments },
        },
      ],
    };
  }, [
    flightStatus?.simconnectConnected,
    flightStatus?.currentLat,
    flightStatus?.currentLon,
    flightStatus?.originIcao,
    simbriefFlights,
  ]);

  // Geometría del track real — sólo se popula cuando hay selección
  // con datos. LineString (no MultiLineString) porque es una traza
  // continua, no varios segmentos.
  const trackGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.LineString>
  >(() => {
    if (selectedFlightId == null || trackPoints.length < 2) {
      return { type: "FeatureCollection", features: [] };
    }
    // (v3.4.0) Sanitizamos con timestamps para detectar pausas
    // reales en el sampling, no sólo saltos geométricos. Sin esto
    // un vuelo con sim pausado 5 min seguía mostrando línea recta
    // entre los dos segmentos — ahora se corta.
    const sanitized = sanitizeTrackCoordsWithTs(
      trackPoints.map((p) => ({ lon: p.lon, lat: p.lat, ts: p.ts })),
    );
    const coords: [number, number][] = sanitized.map(
      (p) => [p.lon, p.lat] as [number, number],
    );
    if (coords.length < 2) {
      return { type: "FeatureCollection", features: [] };
    }
    // (v2.2.0) Suavizado Catmull-Rom — antes el trazo se veía a
    // poligonal en zonas de giro. Con la spline pasamos por los
    // mismos puntos pero con interpolación cúbica entre ellos.
    const smooth = smoothCatmullRom(coords, 8);
    return {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: { flightId: selectedFlightId, points: coords.length },
          geometry: { type: "LineString", coordinates: smooth },
        },
      ],
    };
  }, [selectedFlightId, trackPoints]);

  // Modo detalle: cuando hay selección con track válido, ocultamos
  // las great-circles del resto de vuelos y dejamos sólo el track
  // real. Si la selección no tiene track aún (vuelo viejo sin
  // muestreos), caemos al modo globo para no dejar el mapa vacío.
  const detailMode = selectedFlightId != null && trackPoints.length >= 2;

  // SimBrief: cyan tenue (planes son aspiracionales, no reales).
  // Se oculta en modo detalle para que sólo se vea la traza real.
  const simbriefGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.MultiLineString>
  >(
    () => ({
      type: "FeatureCollection",
      features: !showSimbriefLines || detailMode
        ? []
        : simbriefFlights.map((f) => ({
            type: "Feature",
            properties: {
              ofpId: f.ofpId,
              label: `${f.originIcao} → ${f.destinationIcao}`,
            },
            geometry: {
              type: "MultiLineString",
              coordinates: greatCircleLine(
                f.originLon,
                f.originLat,
                f.destinationLon,
                f.destinationLat,
              ),
            },
          })),
    }),
    [simbriefFlights, showSimbriefLines, detailMode],
  );

  // SimConnect: verde esmeralda con halo — vuelos reales son
  // los que más le importan al usuario, así que pesan más.
  // En modo detalle se oculta (lo reemplaza el track real).
  const flightLogGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.MultiLineString>
  >(
    () => ({
      type: "FeatureCollection",
      features: !showSimconnectLines || detailMode
        ? []
        : flightLogEntries
            .filter(
              (e) =>
                e.endedAt !== null &&
                e.destinationLat !== null &&
                e.destinationLon !== null,
            )
            .map((e) => ({
              type: "Feature",
              properties: {
                id: e.id,
                label: `${e.originIcao ?? "?"} → ${e.destinationIcao ?? "?"}`,
                distanceNm: e.distanceNm,
              },
              geometry: {
                type: "MultiLineString",
                coordinates: greatCircleLine(
                  e.originLon,
                  e.originLat,
                  e.destinationLon as number,
                  e.destinationLat as number,
                ),
              },
            })),
    }),
    [flightLogEntries, showSimconnectLines, detailMode],
  );

  // Marcadores en origen + destino de cada vuelo. Punto pequeño
  // verde (vuelos reales) o cyan (planes). En modo detalle sólo
  // mostramos los endpoints del vuelo seleccionado.
  const endpointsGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.Point>
  >(() => {
    const features: GeoJSON.Feature<GeoJSON.Point>[] = [];
    if (detailMode && selectedFlightId != null) {
      const selected = flightLogEntries.find((e) => e.id === selectedFlightId);
      if (selected) {
        features.push({
          type: "Feature",
          properties: { kind: "real", role: "origin", icao: selected.originIcao },
          geometry: {
            type: "Point",
            coordinates: [selected.originLon, selected.originLat],
          },
        });
        if (
          selected.destinationLat !== null &&
          selected.destinationLon !== null
        ) {
          features.push({
            type: "Feature",
            properties: {
              kind: "real",
              role: "dest",
              icao: selected.destinationIcao,
            },
            geometry: {
              type: "Point",
              coordinates: [selected.destinationLon, selected.destinationLat],
            },
          });
        }
      }
      return { type: "FeatureCollection", features };
    }
    if (showSimconnectLines) {
      for (const e of flightLogEntries) {
        if (e.endedAt === null) continue;
        if (e.destinationLat === null || e.destinationLon === null) continue;
        features.push({
          type: "Feature",
          properties: { kind: "real", role: "origin", icao: e.originIcao },
          geometry: { type: "Point", coordinates: [e.originLon, e.originLat] },
        });
        features.push({
          type: "Feature",
          properties: { kind: "real", role: "dest", icao: e.destinationIcao },
          geometry: {
            type: "Point",
            coordinates: [e.destinationLon, e.destinationLat],
          },
        });
      }
    }
    if (showSimbriefLines) {
      for (const f of simbriefFlights) {
        features.push({
          type: "Feature",
          properties: { kind: "plan", role: "origin", icao: f.originIcao },
          geometry: { type: "Point", coordinates: [f.originLon, f.originLat] },
        });
        features.push({
          type: "Feature",
          properties: { kind: "plan", role: "dest", icao: f.destinationIcao },
          geometry: {
            type: "Point",
            coordinates: [f.destinationLon, f.destinationLat],
          },
        });
      }
    }
    return { type: "FeatureCollection", features };
  }, [
    flightLogEntries,
    simbriefFlights,
    showSimbriefLines,
    showSimconnectLines,
    detailMode,
    selectedFlightId,
  ]);

  // (v1.1.3) Update de data — puro `setData` sobre los sources que
  // ya fueron inicializados en el `on("load")` callback. Cero
  // chequeo de "exists/addSource" porque garantizamos que existen
  // cuando `mapReady=true`. Forzamos `triggerRepaint()` al final
  // para que el cambio se aplique inmediatamente sin esperar al
  // próximo frame de animación (importante cuando hay rapid clicks
  // entre vuelos seleccionados).
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !mapReady) return;
    const simbriefSrc = map.getSource("rt-simbrief") as GeoJSONSource | undefined;
    simbriefSrc?.setData(simbriefGeojson);
    const flightSrc = map.getSource("rt-flightlog") as GeoJSONSource | undefined;
    flightSrc?.setData(flightLogGeojson);
    const epSrc = map.getSource("rt-endpoints") as GeoJSONSource | undefined;
    epSrc?.setData(endpointsGeojson);
    const trackSrc = map.getSource("rt-track") as GeoJSONSource | undefined;
    trackSrc?.setData(trackGeojson);
    const liveSrc = map.getSource("rt-live") as GeoJSONSource | undefined;
    liveSrc?.setData(liveTrackGeojson);
    const projSrc = map.getSource("rt-projection") as GeoJSONSource | undefined;
    projSrc?.setData(projectionGeojson);
    map.triggerRepaint();
  }, [
    mapReady,
    simbriefGeojson,
    flightLogGeojson,
    endpointsGeojson,
    trackGeojson,
    liveTrackGeojson,
    projectionGeojson,
  ]);

  // (v1.1.4) Marker del avión en vivo. Sólo visible cuando SimConnect
  // está conectado y reporta coords reales. Se rota por heading.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !mapReady) return;
    const live =
      flightStatus?.simconnectConnected &&
      flightStatus.currentLat != null &&
      flightStatus.currentLon != null;
    if (!live) {
      if (planeMarkerRef.current) {
        planeMarkerRef.current.remove();
        planeMarkerRef.current = null;
        planeElementRef.current = null;
      }
      return;
    }
    const lat = flightStatus.currentLat as number;
    const lon = flightStatus.currentLon as number;
    const heading = flightStatus.currentHeadingDeg ?? 0;

    if (!planeMarkerRef.current) {
      const el = document.createElement("div");
      el.style.width = "32px";
      el.style.height = "32px";
      el.style.display = "flex";
      el.style.alignItems = "center";
      el.style.justifyContent = "center";
      el.style.filter = "drop-shadow(0 0 6px rgba(251, 191, 36, 0.55))";
      el.style.transition = "transform 250ms linear";
      el.innerHTML = PLANE_SVG;
      planeElementRef.current = el;
      planeMarkerRef.current = new maplibregl.Marker({
        element: el,
        rotationAlignment: "map",
        pitchAlignment: "map",
      })
        .setLngLat([lon, lat])
        .addTo(map);
    } else {
      planeMarkerRef.current.setLngLat([lon, lat]);
    }
    if (planeMarkerRef.current) {
      planeMarkerRef.current.setRotation(heading);
    }
  }, [
    mapReady,
    flightStatus?.simconnectConnected,
    flightStatus?.currentLat,
    flightStatus?.currentLon,
    flightStatus?.currentHeadingDeg,
  ]);

  // Auto-fit bounds. Doble lógica:
  //   1. Modo globo (sin selección): fit a la primera carga de
  //      rutas, después dejamos al usuario pan/zoom libre.
  //   2. Modo detalle: fit al track CADA vez que cambie la
  //      selección — el usuario espera ver el vuelo elegido,
  //      no la vista anterior.
  //
  // (v1.1.3) Depende de `mapReady` para no intentar fitBounds antes
  // de que el style esté listo. fitBounds usa `easeTo` con animación
  // 800ms; rapid switching entre vuelos cancela la animación previa
  // de manera natural en MapLibre.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !mapReady) return;
    const bounds = new maplibregl.LngLatBounds();
    if (detailMode) {
      for (const feat of trackGeojson.features) {
        for (const [lng, lat] of feat.geometry.coordinates) {
          bounds.extend([lng, lat]);
        }
      }
      if (!bounds.isEmpty()) {
        map.fitBounds(bounds, {
          padding: 80,
          maxZoom: 8,
          duration: 800,
        });
      }
      return;
    }
    // Globo: respeta el "una vez" original.
    if (didAutoFitRef.current) return;
    const allFeatures = [
      ...simbriefGeojson.features,
      ...flightLogGeojson.features,
    ];
    if (allFeatures.length === 0) return;
    for (const feat of allFeatures) {
      for (const segment of feat.geometry.coordinates) {
        for (const [lng, lat] of segment) {
          bounds.extend([lng, lat]);
        }
      }
    }
    if (!bounds.isEmpty()) {
      map.fitBounds(bounds, {
        padding: 60,
        maxZoom: 6,
        duration: 800,
      });
      didAutoFitRef.current = true;
    }
  }, [mapReady, simbriefGeojson, flightLogGeojson, trackGeojson, detailMode]);

  const totalReal = flightLogGeojson.features.length;
  const totalPlan = simbriefGeojson.features.length;
  const empty = !detailMode && totalReal === 0 && totalPlan === 0;

  return (
    <div
      className={`relative overflow-hidden rounded-xl border border-slate-800 bg-slate-950 ${
        className ?? ""
      }`}
      style={{ height }}
    >
      <div ref={containerRef} className="absolute inset-0" />
      {/* Leyenda flotante. En modo globo muestra los conteos; en
          modo detalle, sólo el badge ámbar del track real. */}
      <div className="pointer-events-none absolute left-3 top-3 flex flex-wrap gap-2 text-[10px] font-medium uppercase tracking-wide text-slate-300">
        {detailMode ? (
          <span className="rounded-md bg-slate-950/80 px-2 py-1 ring-1 ring-amber-500/30">
            <span className="mr-1 inline-block h-1.5 w-3 rounded-full bg-amber-400 align-middle" />
            Ruta real{" "}
            <span className="text-amber-300">({trackPoints.length} pts)</span>
          </span>
        ) : (
          <>
            <span className="rounded-md bg-slate-950/80 px-2 py-1 ring-1 ring-emerald-500/30">
              <span className="mr-1 inline-block h-1.5 w-3 rounded-full bg-emerald-400 align-middle" />
              Reales <span className="text-emerald-300">({totalReal})</span>
            </span>
            <span className="rounded-md bg-slate-950/80 px-2 py-1 ring-1 ring-sky-500/30">
              <span className="mr-1 inline-block h-1.5 w-3 rounded-full bg-sky-300 align-middle" />
              Plan <span className="text-sky-300">({totalPlan})</span>
            </span>
          </>
        )}
      </div>
      {loadingTrack && detailMode && (
        <div className="pointer-events-none absolute inset-x-0 bottom-3 flex justify-center text-[10px] text-amber-300">
          Cargando track…
        </div>
      )}
      {detailMode && !loadingTrack && trackPoints.length < 2 && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-center text-xs text-slate-400">
          Este vuelo no tiene track grabado (versión anterior a la 0.1.23).
        </div>
      )}
      {empty && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-center text-xs text-slate-500">
          Aún no hay rutas para mostrar.
          <br />
          Vuela en MSFS o refresca SimBrief.
        </div>
      )}
    </div>
  );
}
