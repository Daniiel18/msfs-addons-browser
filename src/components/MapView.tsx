import { useEffect, useMemo, useRef, useState } from "react";
import maplibregl, { type GeoJSONSource } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import {
  AlertCircle,
  Info,
  MapPin,
  Search,
  Sparkles,
} from "lucide-react";
import type { AvailableUpdate, CommunityPackage } from "../lib/types";
import { isAirport } from "../lib/packageType";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { api } from "../lib/tauri";
import { PackageDetailModal } from "./PackageDetailModal";
import { t } from "../lib/i18n";

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
  const focused = useCommunityStore((s) => s.focused);
  const setFocused = useCommunityStore((s) => s.setFocused);
  const detailsFor = useCommunityStore((s) => s.detailsFor);
  const openDetails = useCommunityStore((s) => s.openDetails);
  const lastScanError = useCommunityStore((s) => s.lastScanError);
  // (v4.0.0 — P3.1) Necesitamos el set de ICAOs con GSX al nivel del
  // padre para alimentar el GeoJSON con la prop `hasGsx`. El layer del
  // mapa luego pinta rojo los puntos sin GSX (mismo patrón que el
  // ámbar de updates).
  const gsxInstalledIcaos = useGsxLocalStore((s) => s.installedIcaos);

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

  // Inicializa MapLibre una vez. Auto-fit al mundo entero al
  // mount y en cada resize del contenedor — así el usuario siempre
  // ve el mapamundi completo sin importar que la ventana esté en
  // pantalla completa o reducida.
  useEffect(() => {
    if (!containerRef.current || mapRef.current) return;
    const map = new maplibregl.Map({
      container: containerRef.current,
      style: OSM_STYLE,
      center: [0, 20],
      zoom: 1,
      minZoom: 1,
      attributionControl: { compact: true },
      renderWorldCopies: true,
    });
    map.addControl(
      new maplibregl.NavigationControl({ showCompass: false }),
      "top-right",
    );
    map.dragRotate.disable();
    map.touchZoomRotate.disableRotation();
    mapRef.current = map;

    // Función para encajar el mundo dentro del viewport actual.
    // La invocamos al cargar y cuando el contenedor cambia de
    // tamaño (resize de ventana, fullscreen toggle, etc.).
    const fitWorld = () => {
      const b = new maplibregl.LngLatBounds([-170, -55], [170, 75]);
      map.fitBounds(b, { padding: 30, duration: 0, animate: false });
    };
    map.on("load", fitWorld);

    // ResizeObserver — cuando el contenedor cambia de tamaño,
    // recentramos. Usar `map.resize()` y `fitWorld` mantiene el
    // canvas y el viewport sincronizados.
    const ro = new ResizeObserver(() => {
      map.resize();
      // Sólo re-fit si el usuario no ha hecho zoom in (zoom < 3
      // implica vista mundial). Así no rompemos su navegación.
      if (map.getZoom() < 3) fitWorld();
    });
    ro.observe(containerRef.current);

    return () => {
      ro.disconnect();
      map.remove();
      mapRef.current = null;
    };
  }, []);

  const geojson = useMemo(
    () => buildGeoJSON(geolocated, updatesByFolder, gsxInstalledIcaos),
    [geolocated, updatesByFolder, gsxInstalledIcaos],
  );

  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;

    // Antes hacíamos auto-fit a los datos; ahora preferimos siempre
    // mantener la vista mundial (lo encajamos en el efecto de mount).
    // Si el usuario quiere ir a un aeropuerto concreto usa la
    // sidebar (que llama `easeTo` con zoom alto).
    const fitToData = () => {
      didAutoFitRef.current = true;
    };

    const apply = () => {
      const existing = map.getSource("packages") as GeoJSONSource | undefined;
      if (existing) {
        existing.setData(geojson);
        fitToData();
        return;
      }
      // Clustering deshabilitado — el usuario quiere ver TODOS los
      // markers desde el primer zoom sin tener que acercar. Con ~100
      // puntos repartidos por el mundo el solapamiento es mínimo;
      // los pocos casos donde hay 2-3 aeropuertos pegados (Madrid +
      // Cuatro Vientos, Tokio Haneda + Narita) los dejamos como
      // discos individuales que el usuario puede separar haciendo
      // zoom.
      map.addSource("packages", {
        type: "geojson",
        data: geojson,
      });

      // (v4.0.0 — P3.1) Color del marker según prioridad:
      //   1. **Ámbar** (`#f59e0b`) — hay update disponible (señal más
      //      accionable, sobre-escribe cualquier otro estado).
      //   2. **Rojo** (`#ef4444`) — no tiene perfil GSX local
      //      (oportunidad clara: el usuario puede ir a flightsim.to
      //      a bajarlo). Igual estilo visual que el badge GSX rojo
      //      que ya existe en MapView sidebar.
      //   3. **Verde** (`#10b981`) — al día y con GSX (estado nominal).
      // Stroke blanco para garantizar contraste contra el azul del
      // océano del basemap.
      map.addLayer({
        id: "package-point",
        type: "circle",
        source: "packages",
        paint: {
          "circle-color": [
            "case",
            ["==", ["get", "hasUpdate"], true],
            "#f59e0b", // ámbar — update disponible (prioridad max)
            ["==", ["get", "hasGsx"], false],
            "#ef4444", // rojo — sin perfil GSX
            "#10b981", // verde — al día y con GSX
          ],
          "circle-radius": 6,
          "circle-stroke-width": 1.5,
          "circle-stroke-color": "rgba(255, 255, 255, 0.9)",
        },
      });

      // Click en un punto: enfoca + abre modal de detalle.
      map.on("click", "package-point", (e) => {
        const feat = e.features?.[0];
        if (!feat) return;
        const props = feat.properties as Record<string, string> | null;
        if (!props || !props.folderName) return;
        setFocused(props.folderName);
        useCommunityStore.getState().openDetails(props.folderName);
      });

      map.on("mouseenter", "package-point", () => {
        map.getCanvas().style.cursor = "pointer";
      });
      map.on("mouseleave", "package-point", () => {
        map.getCanvas().style.cursor = "";
      });

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

  // NOTA: las líneas de rutas SimBrief + SimConnect vivían acá
  // antes; las migramos al componente `RoutesMapView` que se monta
  // dentro de `FlightBookView`. El usuario pidió que esta vista
  // sólo muestre escenarios (aeropuertos del catálogo) y que las
  // rutas tengan su propio mapa elegante en FlightBook.

  return (
    // Layout responsive: en pantallas grandes la sidebar crece a 380px
    // y el mapa ocupa el resto (era fijo 320px antes, se quedaba
    // chico). Mantenemos h-screen-minus-chrome para que el mapa
    // siempre llene visiblemente.
    <div className="grid h-[calc(100vh-12rem)] min-h-[520px] grid-cols-[1fr_360px] gap-4 xl:grid-cols-[1fr_400px]">
      <div className="relative overflow-hidden rounded-2xl border border-slate-800 bg-slate-900/40">
        <div ref={containerRef} className="absolute inset-0" />

        <div className="pointer-events-none absolute left-4 top-4 flex flex-col gap-2">
          <div className="pointer-events-auto inline-flex items-center gap-2 rounded-md bg-slate-950/80 px-3 py-1.5 text-xs text-slate-200 backdrop-blur ring-1 ring-slate-800">
            <MapPin className="h-3.5 w-3.5 text-emerald-300" />
            {geolocated.length} {geolocated.length === 1 ? t("map.airport_singular") : t("map.airport_plural")}
            {/* El contador del mapa muestra sólo updates de SCENERY
                (los que sí están pintados como markers). Antes
                contábamos `updates.length` total e incluía AIRCRAFT,
                así que decía "4" cuando en el mapa solo había 1. */}
            {updatesByFolder.size > 0 && (
              <button
                onClick={() => useCommunityStore.getState().startUpdateAll()}
                title={t("map.update_all.tooltip")}
                className="ml-2 inline-flex items-center gap-1 rounded bg-amber-500/20 px-1.5 py-0.5 text-amber-300 ring-1 ring-amber-500/30 hover:bg-amber-500/30 hover:text-amber-200"
              >
                <Sparkles className="h-3 w-3" />
                {updatesByFolder.size} {updatesByFolder.size === 1 ? t("map.update_singular") : t("map.update_plural")}
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
}: {
  packages: CommunityPackage[];
  updatesByFolder: Map<string, AvailableUpdate>;
  updatesCount: number;
  onUpdateAll: () => void;
  focused: string | null;
  onFocus: (folder: string) => void;
  onShowDetails: (folder: string) => void;
}) {
  const [filter, setFilter] = useState("");
  // (v4.0.0 — P2) Filtro GSX. `"all"` = sin filtro (default), `"gsx"` =
  // sólo escenarios con perfil GSX instalado, `"no-gsx"` = sólo los que
  // NO lo tienen. Mutuamente exclusivo: activar uno desactiva el otro.
  // Click en el chip activo lo desactiva y vuelve a `"all"`.
  const [gsxFilter, setGsxFilter] = useState<"all" | "gsx" | "no-gsx">("all");
  // (v2.0.0) Set de ICAOs con perfil GSX local — para badge por
  // escenario en esta lista (no ya sólo en results de búsqueda).
  const gsxInstalledIcaos = useGsxLocalStore((s) => s.installedIcaos);

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    let pool = packages;
    if (q) {
      pool = pool.filter((p) =>
        [p.title, p.creator, p.icao, p.folderName]
          .filter(Boolean)
          .some((s) => s!.toLowerCase().includes(q)),
      );
    }
    if (gsxFilter !== "all") {
      pool = pool.filter((p) => {
        const has = !!p.icao && gsxInstalledIcaos.has(p.icao.toUpperCase());
        return gsxFilter === "gsx" ? has : !has;
      });
    }
    return pool;
  }, [filter, packages, gsxFilter, gsxInstalledIcaos]);

  // Contadores para mostrar dentro de cada chip — ayudan a entender
  // de un vistazo cuántos sceneries caen en cada bucket.
  const { gsxCount, noGsxCount } = useMemo(() => {
    let g = 0;
    let n = 0;
    for (const p of packages) {
      const has = !!p.icao && gsxInstalledIcaos.has(p.icao.toUpperCase());
      if (has) g += 1;
      else n += 1;
    }
    return { gsxCount: g, noGsxCount: n };
  }, [packages, gsxInstalledIcaos]);

  const toggleGsx = (target: "gsx" | "no-gsx") => {
    setGsxFilter((cur) => (cur === target ? "all" : target));
  };

  return (
    <aside className="flex flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-900/40">
      <header className="flex items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
        <div>
          <h3 className="text-sm font-semibold text-slate-100">
            {t("map.installed")} ({packages.length})
          </h3>
        </div>
      </header>

      {updatesCount > 0 && (
        <div className="border-b border-slate-800 bg-amber-500/5 px-3 py-2">
          <button
            onClick={onUpdateAll}
            className="inline-flex w-full items-center justify-center gap-2 rounded-md bg-amber-500 px-3 py-1.5 text-xs font-bold uppercase tracking-wide text-amber-950 shadow-md shadow-amber-500/20 hover:bg-amber-400"
          >
            <Sparkles className="h-3.5 w-3.5" />
            {t("map.update_all")} ({updatesCount})
          </button>
        </div>
      )}

      <div className="space-y-2 border-b border-slate-800 px-3 py-2">
        <div className="relative">
          <Search className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500" />
          <input
            type="text"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={t("map.filter_placeholder")}
            className="w-full rounded-md border border-slate-800 bg-slate-950/50 py-1.5 pl-7 pr-2 text-xs text-slate-200 placeholder:text-slate-500 focus:border-brand-500/40 focus:outline-none focus:ring-1 focus:ring-brand-500/30"
          />
        </div>
        {/* (v4.0.0 — P2) Chips GSX / NO GSX. Mutuamente exclusivos.
            Click en el chip activo lo desactiva → vuelve a "all". */}
        <div className="flex flex-wrap gap-1.5">
          <GsxFilterChip
            active={gsxFilter === "gsx"}
            tone="violet"
            onClick={() => toggleGsx("gsx")}
            label={t("map.gsx_filter.with")}
            count={gsxCount}
          />
          <GsxFilterChip
            active={gsxFilter === "no-gsx"}
            tone="slate"
            onClick={() => toggleGsx("no-gsx")}
            label={t("map.gsx_filter.without")}
            count={noGsxCount}
          />
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {visible.length === 0 && (
          <div className="px-4 py-8 text-center text-xs text-slate-500">
            {packages.length === 0
              ? t("map.empty.no_packages")
              : t("map.empty.no_match")}
          </div>
        )}
        <ul className="divide-y divide-slate-800">
          {visible.map((p) => {
            const update = updatesByFolder.get(p.folderName);
            const isFocused = focused === p.folderName;
            // (v2.0.0) ¿Tiene perfil GSX local para este ICAO? Sólo
            // tiene sentido en sceneries con ICAO definido.
            const hasGsx =
              !!p.icao && gsxInstalledIcaos.has(p.icao.toUpperCase());
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
                  title={t("map.focus_tooltip")}
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
                      {p.icao && hasGsx && (
                        <span
                          className="inline-flex items-center gap-0.5 rounded bg-emerald-500/15 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-emerald-300 ring-1 ring-emerald-500/40"
                          title={`Perfil GSX detectado en %APPDATA%\\Virtuali\\GSX\\MSFS para ${p.icao}`}
                        >
                          ✓ GSX
                        </span>
                      )}
                      {p.icao && !hasGsx && (
                        <a
                          href="#"
                          onClick={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                            // (v2.1.1) URL canónica de búsqueda GSX en
                            // flightsim.to — el `?q=ICAO` filtra dentro
                            // de la subcategoría gsx-pro.
                            const url = `https://flightsim.to/miscellaneous/gsx-pro?q=${encodeURIComponent(
                              p.icao!,
                            )}`;
                            void api.openExternal(url);
                          }}
                          className="inline-flex items-center gap-0.5 rounded bg-rose-500/10 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-rose-300/80 ring-1 ring-rose-500/30 hover:bg-rose-500/20 hover:text-rose-200"
                          title={`Sin perfil GSX local para ${p.icao}. Click → buscar en flightsim.to`}
                        >
                          ✗ GSX
                        </a>
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
                  title={t("map.details_tooltip")}
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
  gsxInstalledIcaos: Set<string>,
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
          // (v4.0.0 — P3.1) `hasGsx` por feature. Si el ICAO no está
          // en el set local de perfiles GSX → false → marker rojo en
          // el paint expression. Sceneries sin ICAO se asumen "OK"
          // (no podemos correlacionar — irían como verdes).
          hasGsx:
            !p.icao || gsxInstalledIcaos.has(p.icao.toUpperCase()),
        },
        geometry: {
          type: "Point",
          coordinates: [p.longitude!, p.latitude!],
        },
      })),
  };
}

/**
 * (v4.0.0 — P2) Chip de filtro GSX / NO GSX en el sidebar de Scenery.
 *
 * UX:
 *   · No activo → borde y bg neutros, count con tipografía pareja.
 *   · Activo (mutuamente exclusivo entre los dos chips):
 *       - "gsx" → tono violet (mismo accent que el badge GSX existente).
 *       - "no-gsx" → tono slate (visualmente "ausencia").
 *   · Click en el chip activo lo desactiva → caller resetea a "all".
 *
 * El count se renderiza dentro de la pill con bg semi-transparente
 * para que se lea contra el accent. Mismo patrón visual que los
 * type-chips de AddonsView.
 */
function GsxFilterChip({
  active,
  tone,
  onClick,
  label,
  count,
}: {
  active: boolean;
  tone: "violet" | "slate";
  onClick: () => void;
  label: string;
  count: number;
}) {
  const activeClass =
    tone === "violet"
      ? "border-violet-500 bg-violet-500/15 text-violet-100"
      : "border-slate-500 bg-slate-500/15 text-slate-100";
  const idleClass =
    "border-slate-800 bg-slate-900/40 text-slate-300 hover:border-slate-700 hover:bg-slate-800/60";
  const countBgActive =
    tone === "violet"
      ? "bg-violet-500/30 text-violet-100"
      : "bg-slate-500/30 text-slate-100";
  const countBgIdle = "bg-slate-800 text-slate-400";
  return (
    <button
      onClick={onClick}
      className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[11px] font-medium transition-colors ${
        active ? activeClass : idleClass
      }`}
    >
      {label}
      <span
        className={`rounded-full px-1.5 text-[10px] font-semibold ${
          active ? countBgActive : countBgIdle
        }`}
      >
        {count}
      </span>
    </button>
  );
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
