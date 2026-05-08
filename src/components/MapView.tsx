import { useEffect, useMemo, useRef, useState } from "react";
import maplibregl, { type GeoJSONSource, type LngLatLike } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import {
  AlertCircle,
  Info,
  Loader2,
  MapPin,
  RefreshCw,
  Search,
  Sparkles,
} from "lucide-react";
import type { AvailableUpdate, CommunityPackage } from "../lib/types";
import { isAirport } from "../lib/packageType";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { PackageDetailModal } from "./PackageDetailModal";
import { SimBriefPanel } from "./SimBriefPanel";

/**
 * Vista de mapa mundial con sidebar lateral.
 *
 * Layout:
 *   ┌──────────────────────────────────────┬──────────────┐
 *   │                                      │  Sidebar:    │
 *   │            MapLibre canvas           │  - lista     │
 *   │  (clusters verdes, popup al click)   │  - search    │
 *   │                                      │  - updates   │
 *   └──────────────────────────────────────┴──────────────┘
 *
 * Fuente de datos: `useCommunityStore` (alimentado por el scanner
 * del backend). El componente no toca la red — sólo escucha los
 * cambios de `packages` y `updates` para repintar.
 */
export function MapView() {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const mapRef = useRef<maplibregl.Map | null>(null);
  // Bandera para hacer un único `fitBounds` automático cuando los
  // datos llegan por primera vez. Sin esto, los marcadores quedaban
  // fuera del viewport inicial y el usuario tenía que hacer zoom out
  // manualmente para verlos.
  const didAutoFitRef = useRef(false);

  const allPackages = useCommunityStore((s) => s.packages);
  const updates = useCommunityStore((s) => s.updates);
  const simbriefFlights = useSimBriefStore((s) => s.flights);
  const focused = useCommunityStore((s) => s.focused);
  const setFocused = useCommunityStore((s) => s.setFocused);
  const detailsFor = useCommunityStore((s) => s.detailsFor);
  const openDetails = useCommunityStore((s) => s.openDetails);
  const rescan = useCommunityStore((s) => s.rescan);
  const scanning = useCommunityStore((s) => s.scanning);
  const refreshUpdates = useCommunityStore((s) => s.refreshUpdates);
  const refreshing = useCommunityStore((s) => s.refreshing);
  const lastScanError = useCommunityStore((s) => s.lastScanError);

  // Aquí sólo viven aeropuertos reales — SCENERY + ICAO resuelto
  // en la tabla `airports` (OurAirports). Eso excluye liveries,
  // sound packs, GSX profile mods, replacements, etc., que el
  // usuario reporta como "no son aeropuertos". Esos pasan a la
  // pestaña "Addons" automáticamente vía `isAddon`.
  const packages = useMemo(
    () => allPackages.filter(isAirport),
    [allPackages],
  );

  // Como `isAirport` ya garantiza coords, el listado del mapa es
  // exactamente `packages`. Mantenemos la variable separada por
  // claridad y por si en el futuro queremos filtrar por viewport.
  const geolocated = packages;

  // Paquete enfocado para centrar la cámara y resaltar la sidebar.
  const focusedPkg = useMemo(
    () => allPackages.find((p) => p.folderName === focused) ?? null,
    [allPackages, focused],
  );
  // Paquete cuyo modal está abierto. Distinto de `focused` porque
  // el modal se abre vía botón explícito, no por click directo.
  const detailsPkg = useMemo(
    () => allPackages.find((p) => p.folderName === detailsFor) ?? null,
    [allPackages, detailsFor],
  );
  const detailsUpdate = useMemo(
    () =>
      detailsFor
        ? updates.find((u) => u.folderName === detailsFor) ?? null
        : null,
    [updates, detailsFor],
  );

  // Sólo las updates de paquetes que están en el mapa (SCENERY +
  // ICAO + coords). Las de AIRCRAFT/MISC viven en la pestaña
  // Addons y no deberían contar en el indicador del mapa.
  const updatesByFolder = useMemo(() => {
    const folderSet = new Set(packages.map((p) => p.folderName));
    const m = new Map<string, AvailableUpdate>();
    for (const u of updates) {
      if (folderSet.has(u.folderName)) m.set(u.folderName, u);
    }
    return m;
  }, [updates, packages]);

  // Inicializa MapLibre una vez.
  useEffect(() => {
    if (!containerRef.current || mapRef.current) return;
    const map = new maplibregl.Map({
      container: containerRef.current,
      style: OSM_STYLE,
      center: [0, 25],
      zoom: 1.4,
      attributionControl: { compact: true },
    });
    map.addControl(
      new maplibregl.NavigationControl({ showCompass: false }),
      "top-right",
    );
    map.dragRotate.disable();
    map.touchZoomRotate.disableRotation();
    mapRef.current = map;
    return () => {
      map.remove();
      mapRef.current = null;
    };
  }, []);

  const geojson = useMemo(
    () => buildGeoJSON(geolocated, updatesByFolder),
    [geolocated, updatesByFolder],
  );

  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;

    const fitToData = () => {
      if (didAutoFitRef.current) return;
      const features = geojson.features;
      if (features.length === 0) return;
      const bounds = new maplibregl.LngLatBounds();
      for (const f of features) {
        const [lng, lat] = (f.geometry as GeoJSON.Point).coordinates;
        bounds.extend([lng, lat]);
      }
      // Padding generoso para que ningún marker quede pegado al
      // borde de la sidebar/controles. `maxZoom` evita que un sólo
      // aeropuerto enfoque a nivel calle — molesto cuando hay 1-2
      // puntos lejanos del resto.
      map.fitBounds(bounds, {
        padding: 60,
        maxZoom: 6,
        duration: 800,
      });
      didAutoFitRef.current = true;
    };

    const apply = () => {
      const existing = map.getSource("packages") as GeoJSONSource | undefined;
      if (existing) {
        existing.setData(geojson);
        fitToData();
        return;
      }
      map.addSource("packages", {
        type: "geojson",
        data: geojson,
        cluster: true,
        // Bajamos el cluster cap a 4 para que la mayoría de
        // marcadores aparezcan individuales desde el primer
        // zoom. Antes era 12 — el usuario tenía que acercar
        // manualmente para ver siquiera los puntos. Ahora
        // sólo se agrupan cuando están literalmente encima a
        // nivel mundial.
        clusterMaxZoom: 4,
        clusterRadius: 50,
      });

      // Clusters
      map.addLayer({
        id: "clusters",
        type: "circle",
        source: "packages",
        filter: ["has", "point_count"],
        paint: {
          "circle-color": [
            "step",
            ["get", "point_count"],
            "#10b981",
            10,
            "#34d399",
            50,
            "#6ee7b7",
          ],
          "circle-radius": [
            "step",
            ["get", "point_count"],
            16,
            10,
            22,
            50,
            28,
          ],
          "circle-stroke-width": 2,
          "circle-stroke-color": "rgba(16, 185, 129, 0.35)",
        },
      });
      map.addLayer({
        id: "cluster-count",
        type: "symbol",
        source: "packages",
        filter: ["has", "point_count"],
        layout: {
          "text-field": "{point_count_abbreviated}",
          "text-size": 12,
          "text-font": ["Open Sans Bold", "Arial Unicode MS Bold"],
        },
        paint: { "text-color": "#022c22" },
      });

      // Punto individual — color depende de "hasUpdate"
      map.addLayer({
        id: "package-point",
        type: "circle",
        source: "packages",
        filter: ["!", ["has", "point_count"]],
        paint: {
          "circle-color": [
            "case",
            ["==", ["get", "hasUpdate"], true],
            "#f59e0b", // ámbar — hay update
            "#10b981", // verde — al día
          ],
          "circle-radius": 7,
          "circle-stroke-width": 2,
          "circle-stroke-color": "rgba(255, 255, 255, 0.85)",
        },
      });

      map.on("click", "clusters", async (e) => {
        const feat = e.features?.[0];
        if (!feat) return;
        const clusterId = feat.properties?.cluster_id as number | undefined;
        if (clusterId == null) return;
        const source = map.getSource("packages") as GeoJSONSource;
        const zoom = await source.getClusterExpansionZoom(clusterId);
        const coords = (feat.geometry as GeoJSON.Point).coordinates as LngLatLike;
        map.easeTo({ center: coords, zoom });
      });

      // Click en un punto: enfoca + abre modal de detalle. El user
      // pidió que click llevara directamente al panel con
      // Reparar/Desinstalar/Abrir carpeta — sin un paso extra
      // ("primero focus, luego pulsar i") que era confuso.
      map.on("click", "package-point", (e) => {
        const feat = e.features?.[0];
        if (!feat) return;
        const props = feat.properties as Record<string, string> | null;
        if (!props || !props.folderName) return;
        setFocused(props.folderName);
        useCommunityStore.getState().openDetails(props.folderName);
      });

      for (const layer of ["clusters", "package-point"] as const) {
        map.on("mouseenter", layer, () => (map.getCanvas().style.cursor = "pointer"));
        map.on("mouseleave", layer, () => (map.getCanvas().style.cursor = ""));
      }

      // Primera vez con layers — encuadrar a los datos.
      fitToData();
    };

    if (map.isStyleLoaded()) apply();
    else map.once("load", apply);
  }, [geojson, setFocused]);

  // Cuando cambia el paquete enfocado (por click en sidebar o
  // marker), centramos la cámara. El modal se renderiza aparte.
  useEffect(() => {
    const map = mapRef.current;
    if (!map || !focusedPkg) return;
    if (focusedPkg.latitude === null || focusedPkg.longitude === null) return;
    const coords: [number, number] = [focusedPkg.longitude, focusedPkg.latitude];
    map.easeTo({ center: coords, zoom: Math.max(map.getZoom(), 9), duration: 600 });
  }, [focusedPkg]);

  // Capa SimBrief — una LineString por vuelo entre origin y dest.
  // Se gestiona aparte del source `packages` para que ambos puedan
  // refrescar independientemente sin recrear el otro.
  const simbriefGeojson = useMemo<GeoJSON.FeatureCollection<GeoJSON.LineString>>(
    () => ({
      type: "FeatureCollection",
      features: simbriefFlights.map((f) => ({
        type: "Feature",
        properties: {
          ofpId: f.ofpId,
          label: `${f.originIcao} → ${f.destinationIcao}`,
        },
        geometry: {
          type: "LineString",
          coordinates: [
            [f.originLon, f.originLat],
            [f.destinationLon, f.destinationLat],
          ],
        },
      })),
    }),
    [simbriefFlights],
  );

  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;
    const apply = () => {
      const existing = map.getSource("simbrief") as GeoJSONSource | undefined;
      if (existing) {
        existing.setData(simbriefGeojson);
        return;
      }
      map.addSource("simbrief", { type: "geojson", data: simbriefGeojson });
      // Primero la línea base ancha (halo), luego la línea
      // brillante encima — efecto "neón" visible sobre OSM.
      map.addLayer({
        id: "simbrief-line-glow",
        type: "line",
        source: "simbrief",
        paint: {
          "line-color": "#0ea5e9",
          "line-width": 5,
          "line-opacity": 0.35,
        },
      });
      map.addLayer({
        id: "simbrief-line",
        type: "line",
        source: "simbrief",
        paint: {
          "line-color": "#7dd3fc",
          "line-width": 2,
          "line-dasharray": [3, 2],
        },
      });
    };
    if (map.isStyleLoaded()) apply();
    else map.once("load", apply);
  }, [simbriefGeojson]);

  return (
    <div className="grid h-[calc(100vh-13rem)] min-h-[480px] grid-cols-[1fr_320px] gap-4">
      <div className="relative overflow-hidden rounded-2xl border border-slate-800 bg-slate-900/40">
        <div ref={containerRef} className="absolute inset-0" />

        <div className="pointer-events-none absolute left-4 top-4 flex flex-col gap-2">
          <div className="pointer-events-auto inline-flex items-center gap-2 rounded-md bg-slate-950/80 px-3 py-1.5 text-xs text-slate-200 backdrop-blur ring-1 ring-slate-800">
            <MapPin className="h-3.5 w-3.5 text-emerald-300" />
            {geolocated.length} aeropuerto{geolocated.length === 1 ? "" : "s"}
            {/* El contador del mapa muestra sólo updates de SCENERY
                (los que sí están pintados como markers). Antes
                contábamos `updates.length` total e incluía AIRCRAFT,
                así que decía "4" cuando en el mapa solo había 1. */}
            {updatesByFolder.size > 0 && (
              <button
                onClick={() => useCommunityStore.getState().startUpdateAll()}
                title="Actualizar todos los aeropuertos con update"
                className="ml-2 inline-flex items-center gap-1 rounded bg-amber-500/20 px-1.5 py-0.5 text-amber-300 ring-1 ring-amber-500/30 hover:bg-amber-500/30 hover:text-amber-200"
              >
                <Sparkles className="h-3 w-3" />
                {updatesByFolder.size} update{updatesByFolder.size === 1 ? "" : "s"}
              </button>
            )}
          </div>
          {lastScanError && (
            <div className="pointer-events-auto inline-flex items-center gap-2 rounded-md bg-rose-500/20 px-3 py-1.5 text-xs text-rose-200 ring-1 ring-rose-500/40">
              <AlertCircle className="h-3.5 w-3.5" />
              {lastScanError}
            </div>
          )}
        </div>
      </div>

      <Sidebar
        packages={packages}
        updatesByFolder={updatesByFolder}
        updatesCount={updatesByFolder.size}
        onUpdateAll={() => useCommunityStore.getState().startUpdateAll()}
        focused={focused}
        onFocus={setFocused}
        onShowDetails={openDetails}
        onRescan={rescan}
        scanning={scanning}
        onRefreshUpdates={refreshUpdates}
        refreshing={refreshing}
      />

      {detailsPkg && (
        <PackageDetailModal
          pkg={detailsPkg}
          update={detailsUpdate}
          onClose={() => openDetails(null)}
        />
      )}
    </div>
  );
}

function Sidebar({
  packages,
  updatesByFolder,
  updatesCount,
  onUpdateAll,
  focused,
  onFocus,
  onShowDetails,
  onRescan,
  scanning,
  onRefreshUpdates,
  refreshing,
}: {
  packages: CommunityPackage[];
  updatesByFolder: Map<string, AvailableUpdate>;
  updatesCount: number;
  onUpdateAll: () => void;
  focused: string | null;
  onFocus: (folder: string) => void;
  onShowDetails: (folder: string) => void;
  onRescan: () => void;
  scanning: boolean;
  onRefreshUpdates: () => void;
  refreshing: boolean;
}) {
  const [filter, setFilter] = useState("");

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return packages;
    return packages.filter((p) =>
      [p.title, p.creator, p.icao, p.folderName]
        .filter(Boolean)
        .some((s) => s!.toLowerCase().includes(q)),
    );
  }, [filter, packages]);

  return (
    <aside className="flex flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-900/40">
      <header className="flex items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
        <div>
          <h3 className="text-sm font-semibold text-slate-100">
            Instalados ({packages.length})
          </h3>
          <p className="text-[11px] text-slate-500">Carpeta Community</p>
        </div>
        <div className="flex gap-1">
          <button
            onClick={onRefreshUpdates}
            disabled={refreshing}
            title="Buscar actualizaciones contra cada fuente"
            className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-50"
          >
            {refreshing ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Sparkles className="h-4 w-4" />
            )}
          </button>
          <button
            onClick={onRescan}
            disabled={scanning}
            title="Re-escanear la carpeta Community"
            className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-50"
          >
            {scanning ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <RefreshCw className="h-4 w-4" />
            )}
          </button>
        </div>
      </header>

      {updatesCount > 0 && (
        <div className="border-b border-slate-800 bg-amber-500/5 px-3 py-2">
          <button
            onClick={onUpdateAll}
            className="inline-flex w-full items-center justify-center gap-2 rounded-md bg-amber-500 px-3 py-1.5 text-xs font-bold uppercase tracking-wide text-amber-950 shadow-md shadow-amber-500/20 hover:bg-amber-400"
          >
            <Sparkles className="h-3.5 w-3.5" />
            Actualizar todo ({updatesCount})
          </button>
        </div>
      )}

      <SimBriefPanel />

      <div className="border-b border-slate-800 px-3 py-2">
        <div className="relative">
          <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
          <input
            type="text"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="Filtrar…"
            className="w-full rounded-md border border-slate-800 bg-slate-950/50 py-1.5 pl-7 pr-2 text-xs text-slate-200 placeholder:text-slate-500 focus:border-brand-500/40 focus:outline-none focus:ring-1 focus:ring-brand-500/30"
          />
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {visible.length === 0 && (
          <div className="px-4 py-8 text-center text-xs text-slate-500">
            {packages.length === 0
              ? "No se encontraron paquetes en Community todavía. Pulsa el botón de re-escaneo arriba si crees que esto es un error."
              : "Ningún paquete coincide con el filtro."}
          </div>
        )}
        <ul className="divide-y divide-slate-800">
          {visible.map((p) => {
            const update = updatesByFolder.get(p.folderName);
            const isFocused = focused === p.folderName;
            return (
              <li
                key={p.folderName}
                className={`group flex items-start gap-2 px-3 py-2.5 transition-colors ${
                  isFocused ? "bg-brand-500/15" : "hover:bg-slate-800/50"
                }`}
              >
                {/* Click principal: enfocar en el mapa. NO abre modal —
                    eso permite al usuario apreciar el zoom sin overlay
                    encima. El modal se abre con el botón "i" a la derecha. */}
                <button
                  onClick={() => onFocus(p.folderName)}
                  className="flex flex-1 items-start gap-2 text-left"
                  title="Click para enfocar este aeropuerto en el mapa"
                >
                  <div className="mt-1 shrink-0">
                    <span
                      className={`inline-block h-2 w-2 rounded-full ${
                        update ? "bg-amber-400" : "bg-emerald-400"
                      }`}
                    />
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      {p.icao && (
                        <span className="font-mono text-[11px] font-semibold text-brand-300">
                          {p.icao}
                        </span>
                      )}
                      {update && (
                        <span className="inline-flex items-center gap-0.5 rounded bg-amber-500/20 px-1.5 py-0.5 text-[10px] font-medium text-amber-300 ring-1 ring-amber-500/30">
                          <Sparkles className="h-2.5 w-2.5" />
                          {update.installedVersion} → {update.latestVersion}
                        </span>
                      )}
                    </div>
                    <div className="mt-0.5 truncate text-xs text-slate-200">
                      {p.title}
                    </div>
                    {p.creator && (
                      <div className="mt-0.5 truncate text-[11px] text-slate-500">
                        {p.creator}
                        {p.packageVersion && ` · v${p.packageVersion}`}
                      </div>
                    )}
                  </div>
                </button>
                {/* Botón secundario: abre el modal de detalles. Visible
                    al hover, separado para no robar el click principal. */}
                <button
                  onClick={() => onShowDetails(p.folderName)}
                  title="Ver detalles, reparar o desinstalar"
                  className="shrink-0 self-center rounded p-1 text-slate-500 opacity-0 transition-opacity hover:bg-slate-800 hover:text-slate-200 group-hover:opacity-100"
                >
                  <Info className="h-3.5 w-3.5" />
                </button>
              </li>
            );
          })}
        </ul>
      </div>
    </aside>
  );
}

function buildGeoJSON(
  packages: CommunityPackage[],
  updatesByFolder: Map<string, AvailableUpdate>,
): GeoJSON.FeatureCollection<GeoJSON.Point> {
  return {
    type: "FeatureCollection",
    features: packages
      .filter((p) => p.latitude !== null && p.longitude !== null)
      .map((p) => ({
        type: "Feature",
        properties: {
          folderName: p.folderName,
          title: p.title,
          icao: p.icao,
          airportName: p.airportName,
          creator: p.creator,
          packageVersion: p.packageVersion,
          hasUpdate: updatesByFolder.has(p.folderName),
        },
        geometry: {
          type: "Point",
          coordinates: [p.longitude!, p.latitude!],
        },
      })),
  };
}

const OSM_STYLE: maplibregl.StyleSpecification = {
  version: 8,
  sources: {
    osm: {
      type: "raster",
      tiles: [
        "https://a.tile.openstreetmap.org/{z}/{x}/{y}.png",
        "https://b.tile.openstreetmap.org/{z}/{x}/{y}.png",
        "https://c.tile.openstreetmap.org/{z}/{x}/{y}.png",
      ],
      tileSize: 256,
      attribution:
        '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
      maxzoom: 19,
    },
  },
  layers: [{ id: "osm", type: "raster", source: "osm" }],
  glyphs: "https://demotiles.maplibre.org/font/{fontstack}/{range}.pbf",
};
