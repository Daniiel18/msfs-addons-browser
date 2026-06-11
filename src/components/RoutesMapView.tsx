import { useEffect, useMemo, useRef, useState } from "react";
import maplibregl, { type GeoJSONSource } from "maplibre-gl";
import { Crosshair, ChevronUp, ChevronDown, X } from "lucide-react";

const tracingLog = (msg: string) =>
  console.info(`[RoutesMapView] ${msg}`);
import "maplibre-gl/dist/maplibre-gl.css";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { greatCircleLine } from "../lib/greatCircle";
import { t } from "../lib/i18n";
import type { RouteFix } from "../lib/types";
import {
  smoothCatmullRom,
  segmentTrackCoords,
} from "../lib/smooth";
import { api } from "../lib/tauri";
import type { FlightTrackPoint } from "../lib/types";
import { buildTerminatorPolygon } from "../lib/terminator";

/** (v4.18.1 → v4.21.0) Bandas del terminator día/noche. Cada banda es el
 *  polígono donde el sol está por debajo de `altitudeDeg`; al apilarlas
 *  la oscuridad crece hacia la noche cerrada (-18°, crepúsculo
 *  astronómico). v4.21.0: opacidades MÁS marcadas — con 0.15 el borde
 *  geométrico (0°) era invisible sobre el basemap satelital y el primer
 *  oscurecimiento perceptible quedaba ~1500 km dentro de la noche, así
 *  que el planeta "parecía" 2-3 horas más temprano (bug reportado).
 *  Ahora la división arranca visible JUSTO en la puesta de sol real
 *  (sincronizada con UTC vía SunCalc) y profundiza a ≈0.57. */
const TERMINATOR_BANDS = [
  { source: "rt-term-0", altitudeDeg: 0, opacity: 0.22 },
  { source: "rt-term-civil", altitudeDeg: -6, opacity: 0.2 },
  { source: "rt-term-nautical", altitudeDeg: -12, opacity: 0.18 },
  { source: "rt-term-astro", altitudeDeg: -18, opacity: 0.16 },
] as const;

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
  // (v4.18.0) Panel de detalle del vuelo EN VIVO (estilo VATSIM-Radar):
  // X lo cierra, ^ lo minimiza, ⌖ centra el mapa en el avión, y un click
  // en el icono del avión del mapa lo reabre.
  const [livePanelOpen, setLivePanelOpen] = useState(true);
  const [livePanelMin, setLivePanelMin] = useState(false);
  const liveReopenRef = useRef<() => void>(() => {});
  liveReopenRef.current = () => {
    setLivePanelOpen(true);
    setLivePanelMin(false);
  };
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
    // (v3.5.0) NavigationControl removido — el usuario reportó que
    // los cuadritos del zoom interferían visualmente con la card de
    // detalle del FlightBook. Zoom via scroll-wheel + pan via drag
    // siguen funcionando nativamente sin necesidad del control.
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
      // FlightLog — verde esmeralda con halo (vuelos reales).
      // (v3.6.3 fix J4) Vuelta a opacity fija — el filtro de aerolínea
      // ahora EXCLUYE features no-matching en lugar de atenuarlas.
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
      // (v3.5.0 F2) Great-circle del vuelo SELECCIONADO cuando NO hay
      // track real (típico de imports de VAS-ACARS / MSFS logbook que
      // no traen sampling de posición). Dibujamos la línea gran-circular
      // origen→destino en ámbar dasheado para que el usuario "vea" la
      // ruta aproximada del vuelo aunque no haya track real. Se renderiza
      // por encima de las great-circles del flightlog (verde) para que
      // destaque visualmente al seleccionar.
      map.addSource("rt-selected-gc", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-selected-gc-glow",
        type: "line",
        source: "rt-selected-gc",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#fbbf24",
          "line-width": 7,
          "line-opacity": 0.25,
        },
      });
      map.addLayer({
        id: "rt-selected-gc-line",
        type: "line",
        source: "rt-selected-gc",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#fbbf24",
          "line-width": 2.4,
          "line-dasharray": [3, 1.5],
          "line-opacity": 0.95,
        },
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
      // (v4.13.0) Se eliminó la línea proyectada great-circle (rt-projection,
      // cyan): era redundante con la ruta real del navlog y confundía
      // (dos líneas a la vez). Ahora solo se dibuja la ruta planificada.

      // (v4.10.0) RUTA PLANIFICADA de SimBrief (navlog): línea violeta a
      // través de los waypoints + puntos + etiquetas. Las etiquetas usan
      // colisión automática (text-allow-overlap:false) y minzoom para
      // declutter: en zoom bajo sólo se ven los nombres que caben.
      map.addSource("rt-plan", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-plan-line",
        type: "line",
        source: "rt-plan",
        layout: { "line-cap": "round", "line-join": "round" },
        paint: {
          "line-color": "#a78bfa",
          "line-width": 1.6,
          "line-opacity": 0.7,
        },
      });
      map.addSource("rt-plan-fixes", { type: "geojson", data: empty });
      map.addLayer({
        id: "rt-plan-fixes-dots",
        type: "circle",
        source: "rt-plan-fixes",
        minzoom: 4,
        paint: {
          "circle-radius": 2.4,
          "circle-color": "#c4b5fd",
          "circle-stroke-width": 1,
          "circle-stroke-color": "#4c1d95",
          "circle-opacity": 0.9,
        },
      });
      map.addLayer({
        id: "rt-plan-fixes-labels",
        type: "symbol",
        source: "rt-plan-fixes",
        minzoom: 5,
        layout: {
          "text-field": ["get", "ident"],
          "text-size": 10,
          "text-offset": [0, -0.9],
          "text-allow-overlap": false,
          "text-optional": true,
          "text-padding": 2,
          "symbol-sort-key": ["get", "sort"],
        },
        paint: {
          "text-color": "#ddd6fe",
          "text-halo-color": "#1e1b4b",
          "text-halo-width": 1.2,
        },
      });

      // (v4.0.0 — P6 → v4.18.1) Terminator día/noche con gradiente
      // REAL de 4 bandas (sol a 0° / -6° / -12° / -18°, los crepúsculos
      // astronómicos estándar). La versión anterior pintaba el polígono
      // del terminator geométrico (0°) con opacidad 0.62 — un borde duro
      // exactamente en la línea de salida del sol, que hacía ver "de
      // noche" zonas donde el sol acababa de salir y movía visualmente
      // la división varios grados (feedback usuario: en RD a las 6 am
      // parecía que ya eran las 8-9). Geometría correcta: el polígono
      // de -18° (noche cerrada) es el MÁS CHICO y está contenido en el
      // de 0° — apilando los 4 con opacidades crecientes hacia adentro
      // se obtiene la transición suave estilo Windy: amanecer/atardecer
      // apenas sombreados, noche profunda oscura.
      //
      // **City lights removidas** (feedback usuario): el dataset
      // puntual con dots ámbar se veía artificial. Se queda para v4.x
      // si el usuario insiste.
      const now0 = new Date();
      for (const band of TERMINATOR_BANDS) {
        map.addSource(band.source, {
          type: "geojson",
          data: buildTerminatorPolygon(now0, 2, band.altitudeDeg),
        });
        map.addLayer({
          id: `${band.source}-fill`,
          type: "fill",
          source: band.source,
          paint: {
            "fill-color": "#020617",
            "fill-opacity": band.opacity,
            "fill-antialias": true,
          },
        });
      }

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

  // (v4.0.0 — P6) Refresca las 4 bandas del terminator cada 60s para
  // que se muevan con la rotación de la Tierra (0.25°/min). Todas se
  // recomputan a partir del mismo `Date` para mantenerse sincronizadas.
  useEffect(() => {
    if (!mapReady) return;
    const map = mapRef.current;
    if (!map) return;
    const refresh = () => {
      try {
        const now = new Date();
        for (const band of TERMINATOR_BANDS) {
          const src = map.getSource(band.source) as GeoJSONSource | undefined;
          if (src) src.setData(buildTerminatorPolygon(now, 2, band.altitudeDeg));
        }
      } catch (e) {
        console.warn("[RoutesMapView] terminator refresh falló:", e);
      }
    };
    refresh();
    // (v4.21.0) 30s (antes 60s) — la Tierra rota 0.25°/min; con 30s el
    // terminator queda siempre a <0.13° de su posición UTC real.
    const id = window.setInterval(refresh, 30_000);
    return () => window.clearInterval(id);
  }, [mapReady]);

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
    // (v4.19.1) 5s (antes 10s): con el muestreo de tierra cada 2s el
    // rodaje se ve casi en vivo y el chord live↔poleado queda corto.
    const t = setInterval(tick, 5_000);
    return () => {
      cancelled = true;
      clearInterval(t);
    };
  }, [inFlightId]);

  // GeoJSON del live track — añadimos la posición actual de
  // SimConnect (más fresca que el polling 10s) como último punto.
  const liveTrackGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.MultiLineString>
  >(() => {
    if (inFlightId == null) {
      return { type: "FeatureCollection", features: [] };
    }
    // (v4.13.0 → v4.19.1) No dibujar NADA durante pushback NI en las
    // fases previas (preflight/engine_running): con GSX el remolque
    // arranca antes de que la máquina de fases diga "pushback" y la
    // línea aparecía igual (bug reportado por segunda vez).
    const ph = flightStatus?.phaseLabel;
    if (ph === "pushback" || ph === "preflight" || ph === "engine_running") {
      return { type: "FeatureCollection", features: [] };
    }
    // (v4.19.1) El live track ARRANCA EN EL TAXI: descartamos el tramo
    // inicial de baja velocidad (cluster del spawn en el gate, remolque
    // y pushback, gs < 8 kt). Ese tramo dibujaba rectas/diagonales desde
    // el gate apenas empezaba el rodaje. El track COMPLETO (incl. la
    // curva del pushback) sigue visible al abrir el vuelo terminado.
    let firstTaxi = 0;
    while (
      firstTaxi < liveTrackPoints.length &&
      (liveTrackPoints[firstTaxi].gsKt ?? 0) < 8
    ) {
      firstTaxi++;
    }
    // (v3.4.0) Llevamos timestamp al sanitizador para detectar gaps
    // de tiempo reales (sim pausado, reconexión) además de los saltos
    // geométricos. Esto resuelve la "línea recta fea" que el usuario
    // reportó cuando el flight log tenía un segmento posterior a una
    // pausa de varios minutos.
    const rawPoints = liveTrackPoints.slice(firstTaxi).map((p) => ({
      lon: p.lon,
      lat: p.lat,
      ts: p.ts,
    }));
    // Append posición live si el watcher reporta coords — PERO solo si
    // está razonablemente cerca del último punto poleado (≤ ~0.06° ≈
    // 3.5 NM, cubre el gap de polling de 10 s incluso en crucero).
    // (v4.19.1) Antes se anexaba SIEMPRE: con el track poleado stale el
    // chord recto "rotaba" desde el gate siguiendo al avión.
    if (
      flightStatus?.simconnectConnected &&
      flightStatus.currentLat != null &&
      flightStatus.currentLon != null
    ) {
      const last = rawPoints[rawPoints.length - 1];
      const closeEnough =
        last != null &&
        Math.abs(last.lon - flightStatus.currentLon) < 0.06 &&
        Math.abs(last.lat - flightStatus.currentLat) < 0.06;
      if (closeEnough) {
        rawPoints.push({
          lon: flightStatus.currentLon,
          lat: flightStatus.currentLat,
          ts: new Date().toISOString(),
        });
      }
    }
    // (v3.23.0) Segmentamos en saltos/gaps → MultiLineString (cada
    // tramo suavizado por separado). Evita la recta larga cuando hay
    // pausa/reconexión o un punto teleportado.
    const lines = segmentTrackCoords(rawPoints)
      .filter((seg) => seg.length >= 2)
      .map((seg) => smoothCatmullRom(seg, 8));
    if (lines.length === 0) {
      return { type: "FeatureCollection", features: [] };
    }
    return {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: { flightId: inFlightId, live: true },
          geometry: { type: "MultiLineString", coordinates: lines },
        },
      ],
    };
  }, [inFlightId, liveTrackPoints, flightStatus]);

  // (v4.13.0) RUTA PLANIFICADA (navlog SimBrief). SOLO se dibuja durante
  // un vuelo ACTIVO (FLYING NOW: sin vuelo seleccionado + SimConnect
  // vivo). Los vuelos completados/seleccionados muestran únicamente el
  // track real (ámbar) — antes salían DOS líneas. Además la ruta se va
  // "consumiendo": solo dibujamos el tramo POR DELANTE del avión y
  // conectamos desde su posición actual.
  const planData = useMemo<{
    line: GeoJSON.FeatureCollection<GeoJSON.MultiLineString>;
    fixes: GeoJSON.FeatureCollection<GeoJSON.Point>;
  }>(() => {
    const emptyLine = {
      type: "FeatureCollection",
      features: [],
    } as GeoJSON.FeatureCollection<GeoJSON.MultiLineString>;
    const emptyFix = {
      type: "FeatureCollection",
      features: [],
    } as GeoJSON.FeatureCollection<GeoJSON.Point>;

    if (selectedFlightId != null) return { line: emptyLine, fixes: emptyFix };
    if (!flightStatus?.simconnectConnected) {
      return { line: emptyLine, fixes: emptyFix };
    }
    const origin = flightStatus.originIcao;
    if (!origin) return { line: emptyLine, fixes: emptyFix };
    const ofp = simbriefFlights.find((f) => f.originIcao === origin);
    const allFixes: RouteFix[] = ofp?.routeFixes ?? [];
    if (allFixes.length < 2) return { line: emptyLine, fixes: emptyFix };

    // Distancia² equirectangular — basta para ordenar/elegir el fix más
    // cercano (no necesitamos km exactos).
    const dist2 = (aLat: number, aLon: number, bLat: number, bLon: number) => {
      const dLat = aLat - bLat;
      const dLon =
        (aLon - bLon) * Math.cos(((aLat + bLat) / 2) * (Math.PI / 180));
      return dLat * dLat + dLon * dLon;
    };

    const curLat = flightStatus.currentLat;
    const curLon = flightStatus.currentLon;
    // (v4.18.1) EN TIERRA la ruta NO se ancla al avión: en pushback/taxi
    // el head dibujaba una diagonal gate→fix que cruzaba el aeropuerto
    // (artefacto reportado). En tierra la ruta arranca en el primer fix
    // del navlog (la pista); solo en el aire conectamos desde el avión.
    const airborne = flightStatus.onGround === false;
    let remaining: RouteFix[] = allFixes;
    let head: [number, number] | null = null;
    if (curLat != null && curLon != null) {
      let bestIdx = 0;
      let best = Infinity;
      for (let i = 0; i < allFixes.length; i++) {
        const d = dist2(curLat, curLon, allFixes[i].lat, allFixes[i].lon);
        if (d < best) {
          best = d;
          bestIdx = i;
        }
      }
      if (airborne) {
        // Si el avión ya pasó el fix más cercano (está más cerca del
        // siguiente que la longitud del tramo), avanzamos uno para no
        // dibujar hacia atrás.
        let startIdx = bestIdx;
        if (bestIdx < allFixes.length - 1) {
          const a = allFixes[bestIdx];
          const b = allFixes[bestIdx + 1];
          if (dist2(curLat, curLon, b.lat, b.lon) < dist2(a.lat, a.lon, b.lat, b.lon)) {
            startIdx = bestIdx + 1;
          }
        }
        remaining = allFixes.slice(startIdx);
        head = [curLon, curLat];
      } else {
        // En tierra: desde el fix más cercano hacia adelante, sin
        // segmento avión→fix. En el gate de salida eso es la ruta
        // completa desde la pista; tras aterrizar, el tramo final.
        remaining = allFixes.slice(bestIdx);
      }
    }
    if (remaining.length < 1) return { line: emptyLine, fixes: emptyFix };

    const pts: Array<[number, number]> = [];
    if (head) pts.push(head);
    for (const fx of remaining) pts.push([fx.lon, fx.lat]);
    const coords: number[][][] = [];
    for (let i = 0; i < pts.length - 1; i++) {
      const segs = greatCircleLine(pts[i][0], pts[i][1], pts[i + 1][0], pts[i + 1][1]);
      for (const s of segs) coords.push(s);
    }
    if (coords.length === 0) return { line: emptyLine, fixes: emptyFix };
    const line: GeoJSON.FeatureCollection<GeoJSON.MultiLineString> = {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: {},
          geometry: { type: "MultiLineString", coordinates: coords },
        },
      ],
    };
    const fixFeatures: GeoJSON.Feature<GeoJSON.Point>[] = remaining.map(
      (fx, i) => ({
        type: "Feature",
        properties: { ident: fx.ident, sidStar: fx.isSidStar, sort: i },
        geometry: { type: "Point", coordinates: [fx.lon, fx.lat] },
      }),
    );
    return {
      line,
      fixes: { type: "FeatureCollection", features: fixFeatures },
    };
  }, [
    selectedFlightId,
    flightStatus?.simconnectConnected,
    flightStatus?.originIcao,
    flightStatus?.currentLat,
    flightStatus?.currentLon,
    flightStatus?.onGround,
    simbriefFlights,
  ]);

  // Geometría del track real del vuelo seleccionado.
  const trackGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.MultiLineString>
  >(() => {
    if (selectedFlightId == null || trackPoints.length < 2) {
      return { type: "FeatureCollection", features: [] };
    }
    // (v3.23.0) Segmentamos en saltos geométricos (>3°) y gaps
    // temporales → MultiLineString. Antes era una sola LineString y los
    // tracks corruptos/duplicados (varias sesiones concatenadas o un
    // punto teleportado) dibujaban una recta larga cruzando el mapa /
    // océano. Ahora cada tramo se dibuja por separado (se superponen =
    // ruta limpia) y los outliers aislados quedan en segmentos de 1
    // punto que no se renderizan. Cada tramo se suaviza con Catmull-Rom.
    const lines = segmentTrackCoords(
      trackPoints.map((p) => ({ lon: p.lon, lat: p.lat, ts: p.ts })),
    )
      .filter((seg) => seg.length >= 2)
      .map((seg) => smoothCatmullRom(seg, 8));
    if (lines.length === 0) {
      return { type: "FeatureCollection", features: [] };
    }
    return {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: { flightId: selectedFlightId, segments: lines.length },
          geometry: { type: "MultiLineString", coordinates: lines },
        },
      ],
    };
  }, [selectedFlightId, trackPoints]);

  // (v3.5.0 F2) Great-circle del vuelo seleccionado cuando NO hay
  // track real (imports MSFS logbook sin sampling de posición, o
  // vuelos viejos pre-v0.1.23 sin flight_log_track).
  //
  // (v3.5.0 F2 v4) Guard adicional: NO se renderiza mientras está
  // cargando el track — evitaba el "flash" de línea recta durante el
  // fetch que el usuario reportó como bug ("linea recta que pasa
  // encima de la ruta real").
  const selectedGreatCircleGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.MultiLineString>
  >(() => {
    if (selectedFlightId == null || trackPoints.length >= 2 || loadingTrack) {
      // Hay track real (≥2 pts) o aún cargando → no dibujar la
      // aproximación; el track real lo reemplazará en milisegundos.
      return { type: "FeatureCollection", features: [] };
    }
    const sel = flightLogEntries.find((e) => e.id === selectedFlightId);
    if (
      !sel ||
      sel.destinationLat == null ||
      sel.destinationLon == null ||
      sel.originLat == null ||
      sel.originLon == null
    ) {
      return { type: "FeatureCollection", features: [] };
    }
    // Skip si las coords son artifacts (0,0) en origen O destino.
    // Un vuelo real en el ecuador tiene lat≈0 pero lon distinto, o
    // viceversa; (0,0) literal es solo el placeholder de "sin coords".
    const isZero = (la: number, lo: number) =>
      Math.abs(la) < 0.01 && Math.abs(lo) < 0.01;
    if (
      isZero(sel.originLat, sel.originLon) ||
      isZero(sel.destinationLat, sel.destinationLon)
    ) {
      return { type: "FeatureCollection", features: [] };
    }
    const segments = greatCircleLine(
      sel.originLon,
      sel.originLat,
      sel.destinationLon,
      sel.destinationLat,
    );
    if (segments.length === 0) {
      return { type: "FeatureCollection", features: [] };
    }
    return {
      type: "FeatureCollection",
      features: [
        {
          type: "Feature",
          properties: { id: sel.id, fallback: true },
          geometry: { type: "MultiLineString", coordinates: segments },
        },
      ],
    };
  }, [selectedFlightId, trackPoints, flightLogEntries, loadingTrack]);

  // Modo detalle: hay selección — sea con track real, con great-circle
  // de fallback, o cargando el track. En cualquier caso ocultamos las
  // great-circles del resto de vuelos para que el foco sea el vuelo
  // seleccionado.
  //
  // (v3.5.0 F2 v4) Incluimos `loadingTrack` para que el detail-mode
  // se active INMEDIATAMENTE al seleccionar — sin esperar al fetch.
  // Antes esperaba a tener trackPoints o great-circle, lo que dejaba
  // las great-circles del flightLog visibles durante el load → con
  // 78 vuelos eso era un lío de líneas amontonadas.
  const detailMode =
    selectedFlightId != null &&
    (trackPoints.length >= 2 ||
      selectedGreatCircleGeojson.features.length > 0 ||
      loadingTrack);

  // SimConnect: verde esmeralda con halo.
  // (v3.6.3 fix J4) Focus EXCLUSIVO: cuando hay airline activa, los
  // vuelos que NO matchean simplemente se EXCLUYEN del GeoJSON (antes
  // se atenuaban con opacity — al usuario no le gustó). El mapa muestra
  // sólo las rutas filtradas, sin ruido visual de las demás.
  const selectedAirline = useFlightLogStore((s) => s.selectedAirline);
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
            .filter((e) => {
              if (!selectedAirline) return true;
              return selectedAirline.icao
                ? e.airlineIcao === selectedAirline.icao
                : e.airlineIcao === null &&
                    e.aircraftAirline === selectedAirline.name;
            })
            .map((e) => {
              return {
                type: "Feature" as const,
                properties: {
                  id: e.id,
                  label: `${e.originIcao ?? "?"} → ${e.destinationIcao ?? "?"}`,
                  distanceNm: e.distanceNm,
                },
                geometry: {
                  type: "MultiLineString" as const,
                  coordinates: greatCircleLine(
                    e.originLon,
                    e.originLat,
                    e.destinationLon as number,
                    e.destinationLat as number,
                  ),
                },
              };
            }),
    }),
    [flightLogEntries, showSimconnectLines, detailMode, selectedAirline],
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
    return { type: "FeatureCollection", features };
  }, [
    flightLogEntries,
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
    const flightSrc = map.getSource("rt-flightlog") as GeoJSONSource | undefined;
    flightSrc?.setData(flightLogGeojson);
    const epSrc = map.getSource("rt-endpoints") as GeoJSONSource | undefined;
    epSrc?.setData(endpointsGeojson);
    const trackSrc = map.getSource("rt-track") as GeoJSONSource | undefined;
    trackSrc?.setData(trackGeojson);
    const selGcSrc = map.getSource("rt-selected-gc") as GeoJSONSource | undefined;
    selGcSrc?.setData(selectedGreatCircleGeojson);
    const liveSrc = map.getSource("rt-live") as GeoJSONSource | undefined;
    liveSrc?.setData(liveTrackGeojson);
    const planSrc = map.getSource("rt-plan") as GeoJSONSource | undefined;
    planSrc?.setData(planData.line);
    const planFixSrc = map.getSource("rt-plan-fixes") as
      | GeoJSONSource
      | undefined;
    planFixSrc?.setData(planData.fixes);
    map.triggerRepaint();
  }, [
    mapReady,
    flightLogGeojson,
    endpointsGeojson,
    trackGeojson,
    selectedGreatCircleGeojson,
    liveTrackGeojson,
    planData,
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
      // (v4.18.0) Click en el avión → reabre el panel de detalle en vivo.
      el.style.cursor = "pointer";
      el.addEventListener("click", (ev) => {
        ev.stopPropagation();
        liveReopenRef.current();
      });
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
      // Fit a track real si lo hay, sino a la great-circle de fallback.
      // (v3.23.0) trackGeojson ahora es MultiLineString → iteramos
      // línea→punto (un nivel más de anidación).
      for (const feat of trackGeojson.features) {
        for (const line of feat.geometry.coordinates) {
          for (const [lng, lat] of line) {
            bounds.extend([lng, lat]);
          }
        }
      }
      if (bounds.isEmpty()) {
        for (const feat of selectedGreatCircleGeojson.features) {
          for (const segment of feat.geometry.coordinates) {
            for (const [lng, lat] of segment) {
              bounds.extend([lng, lat]);
            }
          }
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
    const allFeatures = [...flightLogGeojson.features];
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
  }, [
    mapReady,
    flightLogGeojson,
    trackGeojson,
    selectedGreatCircleGeojson,
    detailMode,
  ]);

  // (v3.6.6 fix M2) Re-fit del mapa cuando cambia el filtro de aerolínea.
  // Sin esto, el filter se aplicaba pero la cámara no se movía y daba
  // sensación de que "no pasa nada" — sólo se veía el cambio al cambiar
  // de pantalla y volver. Ahora hace un zoom suave a las rutas filtradas.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !mapReady || detailMode) return;
    // Sólo re-fit si hay un filtro ACTIVO. Si selectedAirline = null,
    // dejamos la vista actual.
    if (!selectedAirline) return;
    const bounds = new maplibregl.LngLatBounds();
    for (const feat of flightLogGeojson.features) {
      for (const segment of feat.geometry.coordinates) {
        for (const [lng, lat] of segment) {
          bounds.extend([lng, lat]);
        }
      }
    }
    if (!bounds.isEmpty()) {
      map.fitBounds(bounds, {
        padding: 80,
        maxZoom: 7,
        duration: 600,
      });
    }
  }, [mapReady, selectedAirline, flightLogGeojson, detailMode]);

  const totalReal = flightLogGeojson.features.length;
  const empty = !detailMode && totalReal === 0;

  return (
    <div
      className={`relative overflow-hidden rounded-xl border border-slate-800 bg-slate-950 ${
        className ?? ""
      }`}
      style={{ height }}
    >
      <div ref={containerRef} className="absolute inset-0" />
      {/* Leyenda flotante. En modo globo muestra los conteos; en
          modo detalle, sólo el badge ámbar del track real.
          (v3.5.0) Movida a top-RIGHT — la card de detalle vive en
          el top-left, así que los chips antes quedaban tapados. */}
      <div className="pointer-events-none absolute right-3 top-3 flex flex-wrap justify-end gap-2 text-[10px] font-medium uppercase tracking-wide text-slate-300">
        {detailMode ? (
          <span className="rounded-md bg-slate-950/80 px-2 py-1 ring-1 ring-amber-500/30">
            <span className="mr-1 inline-block h-1.5 w-3 rounded-full bg-amber-400 align-middle" />
            {t("fb.map.real_track")}{" "}
            <span className="text-amber-300">({trackPoints.length} pts)</span>
          </span>
        ) : (
          <>
            <span className="rounded-md bg-slate-950/80 px-2 py-1 ring-1 ring-emerald-500/30">
              <span className="mr-1 inline-block h-1.5 w-3 rounded-full bg-emerald-400 align-middle" />
              {t("fb.map.real_count")} <span className="text-emerald-300">({totalReal})</span>
            </span>
          </>
        )}
      </div>
      {loadingTrack && detailMode && (
        <div className="pointer-events-none absolute inset-x-0 bottom-3 flex justify-center text-[10px] text-amber-300">
          {t("fb.map.loading_track")}
        </div>
      )}
      {detailMode && !loadingTrack && trackPoints.length < 2 && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-center text-xs text-slate-400">
          {t("fb.map.no_track")}
        </div>
      )}
      {empty && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center text-center text-xs text-slate-500">
          {t("fb.map.empty_title")}
          <br />
          {t("fb.map.empty_hint")}
        </div>
      )}
      {/* (v4.18.0) Panel de detalle del vuelo EN VIVO (estilo VATSIM-Radar).
          Visible solo con vuelo activo (SimConnect + sin vuelo seleccionado).
          X cierra · ^ minimiza · ⌖ centra en el avión · click en el avión
          del mapa lo reabre. */}
      {selectedFlightId == null &&
        flightStatus?.simconnectConnected &&
        flightStatus.originIcao &&
        livePanelOpen && (
          <LiveFlightPanel
            status={flightStatus}
            ofp={simbriefFlights.find(
              (f) => f.originIcao === flightStatus.originIcao,
            )}
            minimized={livePanelMin}
            onToggleMin={() => setLivePanelMin((v) => !v)}
            onClose={() => setLivePanelOpen(false)}
            onCenter={() => {
              const map = mapRef.current;
              if (
                map &&
                flightStatus.currentLat != null &&
                flightStatus.currentLon != null
              ) {
                map.easeTo({
                  center: [flightStatus.currentLon, flightStatus.currentLat],
                  zoom: Math.max(map.getZoom(), 8),
                  duration: 600,
                });
              }
            }}
          />
        )}
    </div>
  );
}

/** (v4.18.0) Distancia great-circle en NM (haversine). */
function gcNm(lat1: number, lon1: number, lat2: number, lon2: number): number {
  const R_NM = 3440.065;
  const toRad = (d: number) => (d * Math.PI) / 180;
  const dLat = toRad(lat2 - lat1);
  const dLon = toRad(lon2 - lon1);
  const a =
    Math.sin(dLat / 2) ** 2 +
    Math.cos(toRad(lat1)) * Math.cos(toRad(lat2)) * Math.sin(dLon / 2) ** 2;
  return 2 * R_NM * Math.asin(Math.min(1, Math.sqrt(a)));
}

/** (v4.18.0) Panel flotante de detalle del vuelo en vivo — estilo
 *  VATSIM-Radar: progreso origen→destino con NM recorridas/restantes y
 *  ETA en HORA LOCAL del PC (regla de la app: nunca UTC), más cards de
 *  GS / altitud / rumbo y los nombres de los aeropuertos. */
function LiveFlightPanel({
  status,
  ofp,
  minimized,
  onToggleMin,
  onClose,
  onCenter,
}: {
  status: import("../lib/types").FlightStatus;
  ofp: import("../lib/types").SimBriefFlight | undefined;
  minimized: boolean;
  onToggleMin: () => void;
  onClose: () => void;
  onCenter: () => void;
}) {
  const lat = status.currentLat;
  const lon = status.currentLon;
  // Progreso por great-circle contra las coords del OFP (si hay). El %
  // usa recorrida/(recorrida+restante) — robusto aunque el avión desvíe.
  let doneNm: number | null = null;
  let remainNm: number | null = null;
  if (ofp && lat != null && lon != null) {
    doneNm = gcNm(ofp.originLat, ofp.originLon, lat, lon);
    remainNm = gcNm(lat, lon, ofp.destinationLat, ofp.destinationLon);
  }
  const pct =
    doneNm != null && remainNm != null && doneNm + remainNm > 0
      ? Math.min(100, (doneNm / (doneNm + remainNm)) * 100)
      : null;
  // ETA en HORA LOCAL del PC: restante / GS.
  const gs = status.currentGroundSpeedKt;
  let etaLocal: string | null = null;
  if (remainNm != null && gs != null && gs > 30) {
    const ms = (remainNm / gs) * 3600 * 1000;
    etaLocal = new Date(Date.now() + ms).toLocaleTimeString([], {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  const phase = (status.phaseLabel ?? "").replace(/_/g, " ");

  return (
    <div className="absolute bottom-3 right-3 z-10 w-[300px] rounded-xl border border-slate-700/80 bg-slate-950/90 p-3 shadow-2xl ring-1 ring-slate-800/70 backdrop-blur">
      {/* Botonera ⌖ ^ X */}
      <div className="absolute right-2 top-2 flex items-center gap-1">
        <button
          onClick={onCenter}
          title={t("fb.live.center")}
          className="rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
        >
          <Crosshair className="h-3.5 w-3.5" />
        </button>
        <button
          onClick={onToggleMin}
          title={minimized ? t("fb.live.expand") : t("fb.live.minimize")}
          className="rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
        >
          {minimized ? (
            <ChevronDown className="h-3.5 w-3.5" />
          ) : (
            <ChevronUp className="h-3.5 w-3.5" />
          )}
        </button>
        <button
          onClick={onClose}
          title={t("common.close")}
          className="rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* (v4.18.1) Header: ICAO - ICAO estado (feedback usuario; antes
          la fase iba en medio de los dos ICAO). */}
      <div className="flex items-center gap-2 pr-20">
        <span className="font-mono text-sm font-bold text-slate-100">
          {status.originIcao ?? "—"} - {status.destinationIcao ?? "—"}
        </span>
        <span className="text-[10px] font-semibold uppercase tracking-wide text-sky-300">
          {phase || "—"}
        </span>
      </div>

      {!minimized && (
        <>
          {/* Barra de progreso con avión */}
          {pct != null && (
            <div className="relative mt-2 h-1.5 w-full rounded-full bg-slate-800">
              <div
                className="h-full rounded-full bg-sky-400"
                style={{ width: `${pct}%` }}
              />
              <span
                className="absolute -top-[5px] text-[11px] leading-none text-sky-200"
                style={{ left: `calc(${pct}% - 6px)` }}
              >
                ✈
              </span>
            </div>
          )}
          {(doneNm != null || remainNm != null) && (
            <div className="mt-1.5 flex items-center justify-between font-mono text-[10px] text-slate-400">
              <span>
                {doneNm != null ? `${Math.round(doneNm)} NM` : "—"}
              </span>
              <span>
                {remainNm != null ? `${Math.round(remainNm)} NM` : "—"}
                {etaLocal ? ` · ETA ${etaLocal}` : ""}
              </span>
            </div>
          )}

          {/* Cards GS / Alt / HDG */}
          <div className="mt-2 grid grid-cols-3 gap-1.5">
            <div className="rounded-lg bg-slate-900/70 px-2 py-1.5 text-center">
              <div className="text-[9px] uppercase tracking-wide text-slate-500">
                GS
              </div>
              <div className="font-mono text-xs text-slate-100">
                {gs != null ? `${Math.round(gs)} kt` : "—"}
              </div>
            </div>
            <div className="rounded-lg bg-slate-900/70 px-2 py-1.5 text-center">
              <div className="text-[9px] uppercase tracking-wide text-slate-500">
                {t("fb.live.altitude")}
              </div>
              <div className="font-mono text-xs text-slate-100">
                {status.currentAltFt != null
                  ? `${Math.round(status.currentAltFt).toLocaleString()} ft`
                  : "—"}
              </div>
            </div>
            <div className="rounded-lg bg-slate-900/70 px-2 py-1.5 text-center">
              <div className="text-[9px] uppercase tracking-wide text-slate-500">
                {t("fb.live.heading")}
              </div>
              <div className="font-mono text-xs text-slate-100">
                {status.currentHeadingDeg != null
                  ? `${Math.round(status.currentHeadingDeg)}°`
                  : "—"}
              </div>
            </div>
          </div>

          {/* Aeropuertos + aeronave */}
          <div className="mt-2 grid grid-cols-2 gap-1.5 text-[10px]">
            <div className="rounded-lg bg-slate-900/70 px-2 py-1.5">
              <div className="font-mono font-semibold text-slate-200">
                {status.originIcao ?? "—"}
              </div>
              <div className="truncate text-slate-500">
                {status.originName ?? ""}
              </div>
            </div>
            <div className="rounded-lg bg-slate-900/70 px-2 py-1.5">
              <div className="font-mono font-semibold text-slate-200">
                {status.destinationIcao ?? "—"}
              </div>
              <div className="truncate text-slate-500">
                {status.destinationName ?? ""}
              </div>
            </div>
          </div>
          {status.aircraftIcao && (
            <div className="mt-1.5 text-center font-mono text-[10px] text-slate-500">
              {t("fb.live.aircraft")}: {status.aircraftIcao}
            </div>
          )}
        </>
      )}
    </div>
  );
}
