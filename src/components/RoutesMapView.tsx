import { useEffect, useMemo, useRef } from "react";
import maplibregl, { type GeoJSONSource } from "maplibre-gl";

const tracingLog = (msg: string) =>
  console.info(`[RoutesMapView] ${msg}`);
import "maplibre-gl/dist/maplibre-gl.css";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { useSettingsStore } from "../stores/useSettingsStore";
import { greatCircleLine } from "../lib/greatCircle";

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
}: {
  className?: string;
  /** Altura del canvas en píxeles. Default 360 — encaja arriba de
   *  la tabla del FlightBook sin robarle protagonismo. */
  height?: number;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  const didAutoFitRef = useRef(false);

  const simbriefFlights = useSimBriefStore((s) => s.flights);
  const flightLogEntries = useFlightLogStore((s) => s.entries);
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
    if (map.isStyleLoaded()) ensureGlobe();
    map.on("style.load", ensureGlobe);

    mapRef.current = map;
    return () => {
      map.off("style.load", ensureGlobe);
      map.remove();
      mapRef.current = null;
    };
  }, []);

  // SimBrief: cyan tenue (planes son aspiracionales, no reales).
  const simbriefGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.MultiLineString>
  >(
    () => ({
      type: "FeatureCollection",
      features: !showSimbriefLines
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
    [simbriefFlights, showSimbriefLines],
  );

  // SimConnect: verde esmeralda con halo — vuelos reales son
  // los que más le importan al usuario, así que pesan más.
  const flightLogGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.MultiLineString>
  >(
    () => ({
      type: "FeatureCollection",
      features: !showSimconnectLines
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
    [flightLogEntries, showSimconnectLines],
  );

  // Marcadores en origen + destino de cada vuelo. Punto pequeño
  // verde (vuelos reales) o cyan (planes). No abrimos popups —
  // los datos están en la tabla.
  const endpointsGeojson = useMemo<
    GeoJSON.FeatureCollection<GeoJSON.Point>
  >(() => {
    const features: GeoJSON.Feature<GeoJSON.Point>[] = [];
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
  }, [flightLogEntries, simbriefFlights, showSimbriefLines, showSimconnectLines]);

  // Aplicar todas las capas (líneas + endpoints) en un solo effect
  // para mantener orden de pintado consistente — endpoints encima
  // de las líneas para que sean clickeables/visibles.
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    const apply = () => {
      // SimBrief lines (cyan tenue)
      const simbriefSrc = map.getSource("rt-simbrief") as GeoJSONSource | undefined;
      if (simbriefSrc) {
        simbriefSrc.setData(simbriefGeojson);
      } else {
        map.addSource("rt-simbrief", { type: "geojson", data: simbriefGeojson });
        map.addLayer({
          id: "rt-simbrief-line-glow",
          type: "line",
          source: "rt-simbrief",
          layout: { "line-cap": "round", "line-join": "round" },
          paint: {
            "line-color": "#0ea5e9",
            "line-width": 3.5,
            "line-opacity": 0.25,
          },
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
      }

      // Flight log lines (verde esmeralda con halo blanco)
      const flightSrc = map.getSource("rt-flightlog") as GeoJSONSource | undefined;
      if (flightSrc) {
        flightSrc.setData(flightLogGeojson);
      } else {
        map.addSource("rt-flightlog", { type: "geojson", data: flightLogGeojson });
        map.addLayer({
          id: "rt-flightlog-line-glow",
          type: "line",
          source: "rt-flightlog",
          layout: { "line-cap": "round", "line-join": "round" },
          paint: {
            "line-color": "#34d399",
            "line-width": 5,
            "line-opacity": 0.35,
          },
        });
        map.addLayer({
          id: "rt-flightlog-line",
          type: "line",
          source: "rt-flightlog",
          layout: { "line-cap": "round", "line-join": "round" },
          paint: {
            "line-color": "#34d399",
            "line-width": 2.4,
          },
        });
      }

      // Endpoints (puntos pequeños en origen/destino)
      const epSrc = map.getSource("rt-endpoints") as GeoJSONSource | undefined;
      if (epSrc) {
        epSrc.setData(endpointsGeojson);
      } else {
        map.addSource("rt-endpoints", { type: "geojson", data: endpointsGeojson });
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
      }
    };
    if (map.isStyleLoaded()) apply();
    else map.once("load", apply);
  }, [simbriefGeojson, flightLogGeojson, endpointsGeojson]);

  // Auto-fit bounds — sólo la primera vez que llegan datos. Después
  // dejamos al usuario controlar el viewport.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || didAutoFitRef.current) return;
    const allFeatures = [
      ...simbriefGeojson.features,
      ...flightLogGeojson.features,
    ];
    if (allFeatures.length === 0) return;
    const bounds = new maplibregl.LngLatBounds();
    for (const feat of allFeatures) {
      for (const segment of feat.geometry.coordinates) {
        for (const [lng, lat] of segment) {
          bounds.extend([lng, lat]);
        }
      }
    }
    if (!bounds.isEmpty()) {
      const fit = () => {
        map.fitBounds(bounds, {
          padding: 60,
          maxZoom: 6,
          duration: 800,
        });
        didAutoFitRef.current = true;
      };
      if (map.isStyleLoaded()) fit();
      else map.once("load", fit);
    }
  }, [simbriefGeojson, flightLogGeojson]);

  const totalReal = flightLogGeojson.features.length;
  const totalPlan = simbriefGeojson.features.length;
  const empty = totalReal === 0 && totalPlan === 0;

  return (
    <div
      className={`relative overflow-hidden rounded-xl border border-slate-800 bg-slate-950 ${
        className ?? ""
      }`}
      style={{ height }}
    >
      <div ref={containerRef} className="absolute inset-0" />
      {/* Leyenda flotante en la esquina superior izquierda. */}
      <div className="pointer-events-none absolute left-3 top-3 flex flex-wrap gap-2 text-[10px] font-medium uppercase tracking-wide text-slate-300">
        <span className="rounded-md bg-slate-950/80 px-2 py-1 ring-1 ring-emerald-500/30">
          <span className="mr-1 inline-block h-1.5 w-3 rounded-full bg-emerald-400 align-middle" />
          Reales <span className="text-emerald-300">({totalReal})</span>
        </span>
        <span className="rounded-md bg-slate-950/80 px-2 py-1 ring-1 ring-sky-500/30">
          <span className="mr-1 inline-block h-1.5 w-3 rounded-full bg-sky-300 align-middle" />
          Plan <span className="text-sky-300">({totalPlan})</span>
        </span>
      </div>
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
