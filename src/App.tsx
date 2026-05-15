import { useEffect, useState } from "react";
import {
  BarChart3,
  Boxes,
  FlaskConical,
  Globe2,
  ListChecks,
  Plane,
  Settings,
} from "lucide-react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api, isTauri } from "./lib/tauri";
import { useAppStore } from "./stores/useAppStore";
import { useDownloadsStore } from "./stores/useDownloadsStore";
import { useCommunityStore } from "./stores/useCommunityStore";
import { useSimBriefStore } from "./stores/useSimBriefStore";
import { useFlightLogStore } from "./stores/useFlightLogStore";
import { useSettingsStore } from "./stores/useSettingsStore";
import { SourceToggle } from "./components/SourceToggle";
import { SearchBar } from "./components/SearchBar";
import { ResultsList } from "./components/ResultsList";
import { DownloadsButton } from "./components/DownloadsButton";
import { DownloadsPanel } from "./components/DownloadsPanel";
import { NotificationsBell } from "./components/NotificationsBell";
import { MapView } from "./components/MapView";
import { AddonsView } from "./components/AddonsView";
import { DashboardView } from "./components/DashboardView";
import { FlightBookView } from "./components/FlightBookView";
import { DragDropOverlay } from "./components/DragDropOverlay";
import { UpdateWizard } from "./components/UpdateWizard";
import { Toaster } from "./components/Toaster";
import { GsxSearchSummary } from "./components/GsxSearchSummary";
import { SettingsModal } from "./components/SettingsModal";
import { SplashScreen, type SplashTask } from "./components/SplashScreen";
import { FlyingNowBadge } from "./components/FlyingNowBadge";
import { OnboardingTour } from "./components/OnboardingTour";

/**
 * Bootstrap centralizado: una sola tarea async awaita Promise.all
 * de los bootstraps de cada store. La splash screen permanece
 * hasta que todos terminan (o fallan, marcados con error). El
 * app principal sólo se monta cuando `ready === true`.
 */
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
  const scanFromFS = useCommunityStore((s) => s.scanFromFS);
  const refreshUpdatesActive = useCommunityStore((s) => s.refreshUpdatesActive);
  const preloadCatalog = useAppStore((s) => s.preloadCatalog);
  const bootstrapSimBrief = useSimBriefStore((s) => s.bootstrap);
  const bootstrapFlightLog = useFlightLogStore((s) => s.bootstrap);
  const bootstrapSettings = useSettingsStore((s) => s.bootstrap);

  const [ready, setReady] = useState(false);
  // Tareas que el splash AWAITEA antes de dar paso a la app.
  // Mantenemos solo las baratas: HTTP "list sources" + DB
  // bootstraps + scan de Community. Las cosas pesadas (pre-cargar
  // catálogos, refrescar updates contra fuentes) corren en
  // background después de que la splash se cierra — así el
  // usuario tiene la app funcional en <10s aunque haya 200+
  // queries de updates pendientes.
  const [splashTasks, setSplashTasks] = useState<SplashTask[]>([
    { label: "Cargar fuentes", status: "pending" },
    { label: "Cargar configuración", status: "pending" },
    { label: "Cargar SimBrief", status: "pending" },
    { label: "Cargar Flight Log", status: "pending" },
    { label: "Suscribir descargas", status: "pending" },
    { label: "Escanear carpeta Community", status: "pending" },
  ]);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [tourOpen, setTourOpen] = useState(false);

  const markTask = (idx: number, status: SplashTask["status"], err?: string) =>
    setSplashTasks((prev) => {
      const next = [...prev];
      next[idx] = { ...next[idx], status, error: err };
      return next;
    });

  useEffect(() => {
    let cancelled = false;
    const run = async () => {
      const wrap = async (idx: number, fn: () => Promise<unknown>) => {
        try {
          await fn();
          if (!cancelled) markTask(idx, "done");
        } catch (e) {
          if (!cancelled) markTask(idx, "error", String(e));
        }
      };

      // FASE 1 — sources + bootstraps de DB en paralelo.
      const sourcesPromise = (async () => {
        const s = await api.listSources();
        if (cancelled) return;
        setSources(s);
        const sourceId =
          s.length && !s.find((x) => x.id === activeSourceId)
            ? s[0].id
            : activeSourceId;
        if (s.length && sourceId !== activeSourceId) {
          setActiveSource(sourceId);
        }
        return s;
      })();
      const sourcesTask = wrap(0, () => sourcesPromise);

      const phase1 = Promise.all([
        sourcesTask,
        wrap(1, () => bootstrapSettings()),
        wrap(2, () => bootstrapSimBrief()),
        wrap(3, () => bootstrapFlightLog()),
        wrap(4, () => bootstrapDownloads()),
      ]);

      // FASE 2 — scan del FS. Independiente de la red. Limitamos
      // a 8 segundos máx para no bloquear el splash si la carpeta
      // Community es muy grande; el scan completo continúa en
      // background si se trunca.
      const phase2 = wrap(5, () =>
        Promise.race([
          scanFromFS(),
          new Promise<void>((resolve) =>
            setTimeout(() => {
              console.warn("scan FS hit 8s timeout, continuing in background");
              resolve();
            }, 8000),
          ),
        ]),
      );

      // Esperamos sólo las fases bloqueantes — splash dura como
      // mucho ~10s (8s scan + ~2s margen). Después dejamos que
      // las tareas pesadas (pre-cargar catálogos + refresh de
      // updates) corran en background sin bloquear al usuario.
      await Promise.allSettled([phase1, phase2]);

      // **Background tasks** — disparadas tras dismissar el splash.
      // Si fallan, no aborta nada; sólo afecta a la frescura de
      // los datos auxiliares.
      void (async () => {
        try {
          const srcs = await sourcesPromise;
          if (srcs && !cancelled) {
            await Promise.all(srcs.map((src) => preloadCatalog(src.id)));
          }
        } catch (e) {
          console.warn("preload catalogs (bg) failed:", e);
        }
        if (!cancelled) {
          refreshUpdatesActive().catch((e) =>
            console.warn("refresh updates (bg) failed:", e),
          );
        }
      })();

      if (!cancelled) {
        await new Promise((r) => setTimeout(r, 350));
        if (cancelled) return;
        if (isTauri) {
          try {
            const win = getCurrentWindow();
            // Devolvemos al modo "app normal": title bar nativo,
            // resizable, tamaño grande, maximizada. El splash arrancó
            // sin decorations (sin X) para que el usuario no pudiera
            // cerrarlo ni minimizarlo durante la carga.
            await win.setDecorations(true);
            await win.setResizable(true);
            await win.setMinSize(new LogicalSize(960, 640));
            await win.setSize(new LogicalSize(1280, 820));
            await win.center();
            await win.maximize();
          } catch (e) {
            console.warn("window resize failed:", e);
          }
        }
        setReady(true);
        // Lanzamos el tour de bienvenida si todavía no se completó
        // ni se saltó esta sesión. Pequeño delay para que la app
        // termine de pintar antes de que el tour mida los rects.
        setTimeout(() => {
          if (cancelled) return;
          const skipped =
            typeof window !== "undefined" &&
            window.sessionStorage.getItem(
              "msfs-addons:onboarding-skip-session",
            );
          const done = useSettingsStore.getState().settings.onboardingCompleted;
          if (!skipped && !done) {
            setTourOpen(true);
          }
        }, 600);
      }
    };

    run().catch((e) => {
      console.error("bootstrap failed:", e);
      setError(String(e));
      setReady(true);
    });

    // Auto-rescan silencioso cuando la ventana recibe foco — la
    // mayoría de usuarios instalan/desinstalan addons fuera de la
    // app (drag-drop a Community, herramientas de terceros) y
    // luego vuelven; queremos que vean su estado real al instante.
    const onFocus = () => {
      // Sólo si el splash ya pasó. Sin esto un focus durante el
      // bootstrap dispararía un scan duplicado.
      if (!cancelled) {
        useCommunityStore
          .getState()
          .scanFromFS()
          .catch((e) => console.warn("focus rescan failed:", e));
      }
    };
    window.addEventListener("focus", onFocus);

    // Listener para el botón "Volver a ver el tour" desde settings.
    const onShowTour = () => setTourOpen(true);
    window.addEventListener(
      "msfs-addons:show-tour" as never,
      onShowTour as never,
    );

    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
      window.removeEventListener(
        "msfs-addons:show-tour" as never,
        onShowTour as never,
      );
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runSearch = async () => {
    if (status === "loading") return;
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

  if (!ready) {
    return <SplashScreen tasks={splashTasks} />;
  }

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

      <header className="sticky top-0 z-10 border-b border-slate-800/80 bg-slate-950/80 backdrop-blur">
        {/* Header truly fluid — sin max-w. Se ajusta al ancho de la
            ventana con padding generoso a los lados. */}
        <div className="flex w-full items-center justify-between px-6 py-3">
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
          <div className="flex items-center gap-2">
            <FlyingNowBadge />
            {view === "search" && (
              <SourceToggle
                sources={sources}
                activeId={activeSourceId}
                onChange={setActiveSource}
              />
            )}
            <button
              data-tour-id="header-settings"
              onClick={() => setSettingsOpen(true)}
              title="Configuración"
              className="rounded-lg border border-slate-800 bg-slate-900/60 p-2 text-slate-400 hover:border-brand-500/40 hover:text-slate-100"
            >
              <Settings className="h-4 w-4" />
            </button>
            <span data-tour-id="header-notifications">
              <NotificationsBell />
            </span>
            <DownloadsButton />
          </div>
        </div>
      </header>

      <DownloadsPanel open={isPanelOpen} onClose={() => setPanelOpen(false)} />

      {/* `main` truly fluid — sin max-w para que el contenido llene
          el ancho disponible en monitores grandes. Sólo Buscar
          mantiene un cap razonable porque las cards muy anchas
          dejan demasiado whitespace en el medio. */}
      <main
        className={`w-full px-6 py-5 ${
          view === "search" ? "mx-auto max-w-[1600px]" : ""
        }`}
      >
        <nav className="mb-5 flex gap-1 rounded-xl bg-slate-900/60 p-1 ring-1 ring-slate-800">
          <ViewTab
            active={view === "dashboard"}
            onClick={() => setView("dashboard")}
            icon={<BarChart3 className="h-4 w-4" />}
            label="Dashboard"
            tourId="nav-dashboard"
          />
          <ViewTab
            active={view === "search"}
            onClick={() => setView("search")}
            icon={<ListChecks className="h-4 w-4" />}
            label="Buscar"
            tourId="nav-search"
          />
          <ViewTab
            active={view === "map"}
            onClick={() => setView("map")}
            icon={<Globe2 className="h-4 w-4" />}
            label="Mapa (escenarios)"
            tourId="nav-map"
          />
          <ViewTab
            active={view === "addons"}
            onClick={() => setView("addons")}
            icon={<Boxes className="h-4 w-4" />}
            label="Addons"
            tourId="nav-addons"
          />
          <ViewTab
            active={view === "flightbook"}
            onClick={() => setView("flightbook")}
            icon={<Plane className="h-4 w-4" />}
            label="FlightBook"
            tourId="nav-flightbook"
          />
        </nav>

        {view === "dashboard" && <DashboardView />}

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
        {view === "flightbook" && <FlightBookView />}
      </main>

      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      {tourOpen && <OnboardingTour onClose={() => setTourOpen(false)} />}
      <DragDropOverlay />
      <UpdateWizard />
      <Toaster />
    </div>
  );
}

function ViewTab({
  active,
  onClick,
  icon,
  label,
  tourId,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
  tourId?: string;
}) {
  return (
    <button
      data-tour-id={tourId}
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
