import { useEffect } from "react";
import { Boxes, FlaskConical, Globe2, ListChecks, Plane } from "lucide-react";
import { api, isTauri } from "./lib/tauri";
import { useAppStore } from "./stores/useAppStore";
import { useDownloadsStore } from "./stores/useDownloadsStore";
import { useCommunityStore } from "./stores/useCommunityStore";
import { SourceToggle } from "./components/SourceToggle";
import { SearchBar } from "./components/SearchBar";
import { ResultsList } from "./components/ResultsList";
import { DownloadsButton } from "./components/DownloadsButton";
import { DownloadsPanel } from "./components/DownloadsPanel";
// `InstallFromFileButton` se eliminó: el drag-and-drop de archivos
// soluciona el mismo flujo y mantiene el header más limpio.
import { NotificationsBell } from "./components/NotificationsBell";
import { UpdateBanner } from "./components/UpdateBanner";
import { MapView } from "./components/MapView";
import { AddonsView } from "./components/AddonsView";
import { DragDropOverlay } from "./components/DragDropOverlay";
import { UpdateWizard } from "./components/UpdateWizard";
import { Toaster } from "./components/Toaster";
import { GsxSearchSummary } from "./components/GsxSearchSummary";

export default function App() {
  const {
    sources, activeSourceId, query, results, status, error, view,
    browsePage, browseHasMore, browseMode,
    setSources, setActiveSource, setQuery, setResults, setStatus, setError, setView,
    loadBrowsePage,
  } = useAppStore();

  const bootstrapDownloads = useDownloadsStore((s) => s.bootstrap);
  const isPanelOpen = useDownloadsStore((s) => s.isPanelOpen);
  const setPanelOpen = useDownloadsStore((s) => s.setPanelOpen);
  const bootstrapCommunity = useCommunityStore((s) => s.bootstrap);

  // Bootstrap: load available sources + subscribe to download events
  // + scan Community + traer updates conocidas. Cada bloque es
  // independiente — un fallo en uno no debe romper los otros.
  useEffect(() => {
    api.listSources()
      .then((s) => {
        setSources(s);
        const sourceId =
          s.length && !s.find((x) => x.id === activeSourceId)
            ? s[0].id
            : activeSourceId;
        if (s.length && sourceId !== activeSourceId) {
          setActiveSource(sourceId);
        } else {
          // Si la fuente activa ya estaba bien, disparamos browse
          // manualmente — `setActiveSource` no se llamó.
          loadBrowsePage(1).catch((e) =>
            console.warn("browse bootstrap failed:", e),
          );
        }
      })
      .catch((e) => setError(String(e)));
    bootstrapDownloads().catch((e) =>
      console.warn("downloads bootstrap failed:", e),
    );
    bootstrapCommunity().catch((e) =>
      console.warn("community bootstrap failed:", e),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runSearch = async () => {
    // Two guards, same spirit: don't stack searches. The UI disables the
    // input while `status === "loading"`, but we duplicate the check here
    // in case the handler is invoked programmatically (e.g. future
    // hotkeys or dev-tools) before the disabled state round-trips.
    if (status === "loading") return;
    // Si el usuario borra el query y pulsa Enter, volvemos al
    // browse-mode con la primera página del catálogo en lugar de
    // dejar la lista vacía con el placeholder.
    if (!query.trim()) {
      loadBrowsePage(1).catch((e) => setError(String(e)));
      return;
    }
    setStatus("loading");
    setError(null);
    try {
      const r = await api.search(query.trim(), activeSourceId);
      setResults(r);
      setStatus("success");
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  };

  return (
    <div className="min-h-screen bg-gradient-to-b from-slate-950 via-slate-950 to-slate-900">
      {!isTauri && (
        <div className="flex items-center justify-center gap-2 border-b border-amber-500/20 bg-amber-500/10 px-4 py-1.5 text-xs text-amber-200">
          <FlaskConical className="h-3.5 w-3.5" />
          <span>
            Modo demo (navegador). Instala Rust y ejecuta{" "}
            <code className="rounded bg-amber-500/20 px-1 font-mono">npm run tauri dev</code>{" "}
            para habilitar la búsqueda real.
          </span>
        </div>
      )}

      <UpdateBanner />

      <header className="sticky top-0 z-10 border-b border-slate-800/80 bg-slate-950/80 backdrop-blur">
        <div className="mx-auto flex max-w-5xl items-center justify-between px-6 py-4">
          <div className="flex items-center gap-2 no-select">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-brand-500/15 ring-1 ring-brand-500/30">
              <Plane className="h-5 w-5 text-brand-300" />
            </div>
            <div>
              <h1 className="text-sm font-semibold tracking-wide text-slate-100">
                MSFS Addons Browser
              </h1>
              <p className="text-[11px] text-slate-500">
                SceneryAddons · Simplaza · GSX
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <SourceToggle
              sources={sources}
              activeId={activeSourceId}
              onChange={setActiveSource}
            />
            <NotificationsBell />
            <DownloadsButton />
          </div>
        </div>
      </header>

      <DownloadsPanel open={isPanelOpen} onClose={() => setPanelOpen(false)} />

      <main className="mx-auto max-w-6xl px-6 py-8">
        <nav className="mb-5 flex gap-1 rounded-xl bg-slate-900/60 p-1 ring-1 ring-slate-800">
          <ViewTab
            active={view === "search"}
            onClick={() => setView("search")}
            icon={<ListChecks className="h-4 w-4" />}
            label="Buscar"
          />
          <ViewTab
            active={view === "map"}
            onClick={() => setView("map")}
            icon={<Globe2 className="h-4 w-4" />}
            label="Mapa (escenarios)"
          />
          <ViewTab
            active={view === "addons"}
            onClick={() => setView("addons")}
            icon={<Boxes className="h-4 w-4" />}
            label="Addons"
          />
        </nav>

        {view === "search" && (
          <>
            <SearchBar
              value={query}
              onChange={setQuery}
              onSubmit={runSearch}
              loading={status === "loading"}
              placeholder={
                activeSourceId === "simplaza"
                  ? "Busca por avión, livery, mod o autor…"
                  : "Busca por aeropuerto, ICAO o desarrollador…"
              }
            />
            {/* Píldora GSX única — sólo visible cuando:
                  · el query es un ICAO de 4 letras, Y
                  · la fuente activa es SceneryAddons (catálogo de
                    aeropuertos al que tiene sentido cruzar GSX).
                Simplaza tiene mods/aviones genéricos, GSX-pro no
                aplica ahí — esconder evita que el usuario crea que
                está relacionado con un resultado de Simplaza. */}
            {activeSourceId === "sceneryaddons" && (
              <div className="mt-3 flex justify-center">
                <GsxSearchSummary query={query} />
              </div>
            )}
            <div className="mt-6">
              <ResultsList
                results={results}
                status={status}
                error={error}
                query={query}
                browseMode={browseMode}
              />
            </div>
            {/* Paginador — sólo visible cuando estamos en browse-mode
                (sin query) y hay resultados. Para los flujos de search
                normal, el catálogo de WordPress no permite página
                navegable porque mete los matches en una sola pantalla
                relevante. */}
            {browseMode === "browse" && results.length > 0 && (
              <div className="mt-4 flex items-center justify-center gap-2">
                <button
                  onClick={() => loadBrowsePage(Math.max(1, browsePage - 1))}
                  disabled={status === "loading" || browsePage <= 1}
                  className="rounded-md border border-slate-800 bg-slate-900/60 px-3 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 hover:text-slate-100 disabled:opacity-40"
                >
                  ← Anterior
                </button>
                <span className="text-xs text-slate-500">
                  Página {browsePage}
                </span>
                <button
                  onClick={() => loadBrowsePage(browsePage + 1)}
                  disabled={status === "loading" || !browseHasMore}
                  className="rounded-md border border-slate-800 bg-slate-900/60 px-3 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 hover:text-slate-100 disabled:opacity-40"
                >
                  Siguiente →
                </button>
              </div>
            )}
          </>
        )}
        {view === "map" && <MapView />}
        {view === "addons" && <AddonsView />}
      </main>

      {/* Overlay global para drag & drop — escucha los eventos de
          Tauri y procesa los archivos sueltos. Vive fuera del flow
          principal para flotar sobre cualquier vista. */}
      <DragDropOverlay />
      {/* Wizard global de "Actualizar todo" — visible cuando la
          cola del community store no está vacía. Se monta aquí
          para flotar sobre cualquier vista. */}
      <UpdateWizard />
      {/* Toasts globales para feedback de descargas/instalaciones. */}
      <Toaster />
    </div>
  );
}

function ViewTab({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      onClick={onClick}
      className={`inline-flex flex-1 items-center justify-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors ${
        active
          ? "bg-brand-500/15 text-brand-200 ring-1 ring-brand-500/30"
          : "text-slate-400 hover:bg-slate-800/60 hover:text-slate-200"
      }`}
    >
      {icon}
      {label}
    </button>
  );
}
