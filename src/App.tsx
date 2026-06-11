import { useEffect, useRef, useState } from "react";
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
import { setActiveLocale, t } from "./lib/i18n";
import { useAppStore } from "./stores/useAppStore";
import { useDownloadsStore } from "./stores/useDownloadsStore";
import { useCommunityStore } from "./stores/useCommunityStore";
import { useSimBriefStore } from "./stores/useSimBriefStore";
import { useFlightLogStore } from "./stores/useFlightLogStore";
import { useSettingsStore } from "./stores/useSettingsStore";
import { useGsxLocalStore } from "./stores/useGsxLocalStore";
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
import { TitleBar } from "./components/TitleBar";
import { DragDropOverlay } from "./components/DragDropOverlay";
import { UpdateWizard } from "./components/UpdateWizard";
import { UpdateBanner } from "./components/UpdateBanner";
import { Toaster } from "./components/Toaster";
import { SimBriefConfirmModal } from "./components/SimBriefConfirmModal";
import { ReplayBanner } from "./components/ReplayBanner";
import { GsxSearchSummary } from "./components/GsxSearchSummary";
import { SettingsModal } from "./components/SettingsModal";
import { SplashScreen, type SplashTask } from "./components/SplashScreen";
import type { UpdateInfo } from "./lib/types";
import { FlyingNowBadge } from "./components/FlyingNowBadge";
import { OnboardingTour } from "./components/OnboardingTour";
import { ImportInventoryModal } from "./components/ImportInventoryModal";

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
  const refreshSimBrief = useSimBriefStore((s) => s.refresh);
  const simBriefPilotId = useSimBriefStore((s) => s.pilotId);
  const bootstrapFlightLog = useFlightLogStore((s) => s.bootstrap);
  const flightStatus = useFlightLogStore((s) => s.status);
  const simRunning = !!flightStatus?.simRunning;
  const bootstrapSettings = useSettingsStore((s) => s.bootstrap);
  const theme = useSettingsStore((s) => s.settings.theme);
  const language = useSettingsStore((s) => s.settings.language);

  // (v3.1.0) Sincroniza el locale activo del módulo i18n con el
  // setting persistido. Cuando el usuario cambia idioma, este efecto
  // re-resuelve y todos los `t()` siguientes usan el nuevo dict (la
  // app pide reload para que componentes con strings inline se
  // actualicen también — controlado por el modal de restart).
  useEffect(() => {
    setActiveLocale((language ?? "auto") as "auto" | "es" | "en");
  }, [language]);

  // (v3.6.0 Phase H — H16) Suscripciones a los eventos de score
  // auto-upload. Cada vuelo que termina dispara `score:done` +
  // `score:upload:success`/`error`. Mostramos toast con el resultado
  // (Q15: "debe mostrarse los logs de que se hizo o si fallo").
  useEffect(() => {
    const unsubs: (() => void)[] = [];
    let cancelled = false;
    void (async () => {
      try {
        const toast = (await import("./stores/useToastStore")).useToastStore;
        const u1 = await api.onScoreUploadSuccess(() => {
          if (cancelled) return;
          toast.getState().push({
            kind: "success",
            title: t("fb.sync.auto_upload_success"),
            ttlMs: 4000,
          });
        });
        const u2 = await api.onScoreUploadError((err) => {
          if (cancelled) return;
          toast.getState().push({
            kind: "error",
            title: t("fb.sync.auto_upload_error", { error: err.error }),
            ttlMs: 10000,
          });
        });
        unsubs.push(u1, u2);
      } catch (e) {
        console.warn("score event subscribe failed:", e);
      }
    })();
    return () => {
      cancelled = true;
      unsubs.forEach((u) => u());
    };
  }, []);

  // Aplicar el tema al `<html>` cada vez que el setting cambie.
  // Tailwind tiene `darkMode: "class"`, así que añadir/quitar la
  // clase `dark` activa/desactiva todos los `dark:` modifiers.
  useEffect(() => {
    const root = document.documentElement;
    if (theme === "light") {
      root.classList.remove("dark");
      root.classList.add("light");
    } else {
      root.classList.remove("light");
      root.classList.add("dark");
    }
  }, [theme]);

  const [ready, setReady] = useState(false);
  // Tareas que el splash AWAITEA antes de dar paso a la app.
  //
  // Cambio v0.1.9: añadimos "Buscar actualización de la app" como
  // primera tarea (si hay update y el usuario acepta, instalamos
  // antes de seguir), y "Buscar actualizaciones de addons" + "Pre-
  // cargar catálogos" al final (que antes corrían en background
  // tras dismiss). Resultado: el usuario abre la app y ve la
  // pestaña Buscar con resultados YA + el bell con notificaciones
  // YA, sin esperas adicionales tras el splash.
  // (v3.4.0) Labels resueltos en render-time vía `t()` para que el
  // idioma del SO o de la preferencia persistida se aplique al
  // splash en el primer paint (no tras el bootstrap).
  const [splashTasks, setSplashTasks] = useState<SplashTask[]>([
    { label: t("splash.task.update_check"), status: "pending" },
    { label: t("splash.task.sources"), status: "pending" },
    { label: t("splash.task.settings"), status: "pending" },
    { label: t("splash.task.simbrief"), status: "pending" },
    { label: t("splash.task.flightlog"), status: "pending" },
    { label: t("splash.task.downloads"), status: "pending" },
    { label: t("splash.task.scan_community"), status: "pending" },
    { label: t("splash.task.refresh_updates"), status: "pending" },
    { label: t("splash.task.preload_catalogs"), status: "pending" },
  ]);

  // Estado del flujo de actualización de la app (auto-update embebido
  // en el splash). Si `appUpdate` es non-null el splash muestra el
  // banner "Hay vX.Y.Z, instalar ahora?". Si el usuario acepta,
  // `installing=true` y `updateProgress` se va llenando con bytes
  // descargados. Cuando termine, el backend hace exit(0) y la app
  // se reabre en la versión nueva. Si el usuario salta, seguimos
  // con el bootstrap normal.
  const [appUpdate, setAppUpdate] = useState<UpdateInfo | null>(null);
  const [updateInstalling, setUpdateInstalling] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<{
    downloadedBytes: number;
    totalBytes: number | null;
  } | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);
  // Promesa que resuelve cuando el usuario decide qué hacer con la
  // update (instalar o saltar). El bootstrap espera por esto.
  const [updateDecision, setUpdateDecision] = useState<{
    resolve: () => void;
  } | null>(null);

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [tourOpen, setTourOpen] = useState(false);
  // (v1.1.4) Modal de importación de inventario. Se abre vía custom
  // event despachado desde la UI de Settings (`ImportInventoryRow`).
  const [importInventoryPath, setImportInventoryPath] = useState<string | null>(
    null,
  );

  const markTask = (idx: number, status: SplashTask["status"], err?: string) =>
    setSplashTasks((prev) => {
      const next = [...prev];
      next[idx] = { ...next[idx], status, error: err };
      return next;
    });

  // Ref para que el bloque async del bootstrap pueda chequear el
  // estado actual de "installing" sin re-renderizar.
  const useInstallingFlagRef = useRef(false);

  // Handler: el usuario pulsó "Instalar ahora" en el splash.
  // Llama al backend; el backend baja el setup, lo lanza silent
  // y hace exit(0). El splash queda mostrando la barra de
  // progreso hasta que la app muera.
  const handleInstallUpdate = async (autoRestart = true) => {
    if (!appUpdate?.assetUrl) return;
    setUpdateInstalling(true);
    useInstallingFlagRef.current = true;
    setUpdateProgress({ downloadedBytes: 0, totalBytes: null });
    try {
      const unsub = await api.onUpdateProgress((p) => setUpdateProgress(p));
      try {
        await api.installUpdate(appUpdate.assetUrl, autoRestart);
      } finally {
        unsub();
      }
    } catch (e) {
      console.warn("installUpdate failed:", e);
      setUpdateInstalling(false);
      useInstallingFlagRef.current = false;
      setUpdateProgress(null);
      // Resolvemos la promesa para que el bootstrap continúe sin
      // update — el usuario verá la app actual.
      updateDecision?.resolve();
      setUpdateDecision(null);
    }
  };

  // Handler: usuario pulsó "Saltar". Continuamos con el bootstrap
  // sin instalar; el banner global (`UpdateBanner`) le seguirá
  // ofreciendo el update dentro de la app.
  const handleSkipUpdate = () => {
    updateDecision?.resolve();
    setUpdateDecision(null);
  };

  // (v2.2.0 → v3.0.0) Auto-refresh SimBrief cuando MSFS está corriendo.
  // El watcher reporta `simRunning` cuando detecta el proceso o
  // SimConnect handshake. Mientras esté true + haya pilotId, pollamos
  // SimBrief cada 2 minutos.
  //
  // **Condición de parada (v3.0.0)** — para no spamear la API si el
  // usuario ya planificó el vuelo, dejamos de pollar cuando el origen
  // del OFP más reciente coincide EXACTAMENTE con el aeropuerto donde
  // está el avión ahora (`currentAirportIcao` desde el nearest del
  // watcher, o `originIcao` si ya hay flight_log abierto). Asumimos
  // "plan sincronizado" y descansamos hasta que cambie el airport.
  const currentAirportIcao =
    flightStatus?.currentAirportIcao ?? flightStatus?.originIcao ?? null;
  const simBriefFlights = useSimBriefStore((s) => s.flights);
  const planSyncedWithAirport = (() => {
    if (!currentAirportIcao) return false;
    if (!simBriefFlights.length) return false;
    // Más reciente primero.
    const sorted = [...simBriefFlights].sort((a, b) => {
      const aTs = a.generatedAt ? parseInt(a.generatedAt, 10) : 0;
      const bTs = b.generatedAt ? parseInt(b.generatedAt, 10) : 0;
      return bTs - aTs;
    });
    return sorted[0]?.originIcao === currentAirportIcao;
  })();

  useEffect(() => {
    if (!ready) return;
    if (!simRunning) return;
    if (!simBriefPilotId) return;
    if (planSyncedWithAirport) {
      // Plan ya sincronizado con el aeropuerto actual — no spameamos.
      return;
    }
    let cancelled = false;
    const tick = () => {
      if (cancelled) return;
      void refreshSimBrief().catch((e) =>
        console.warn("auto-refresh simbrief:", e),
      );
    };
    // Trigger inicial.
    tick();
    const interval = setInterval(tick, 2 * 60_000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [
    ready,
    simRunning,
    simBriefPilotId,
    refreshSimBrief,
    planSyncedWithAirport,
  ]);

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

      // (v2.1.1) Redimensionamos la VENTANA al tamaño del splash al
      // inicio — si el usuario hizo F5 con la app maximizada, el
      // splash estaba flotando en una ventana enorme. Ahora la
      // ventana se encoge al tamaño de la card durante el bootstrap
      // y se expande de vuelta tras `ready=true`.
      if (isTauri) {
        try {
          const win = getCurrentWindow();
          // Quitar maximized si lo está antes de cambiar tamaño,
          // porque setSize sobre una ventana maximizada no hace nada.
          if (await win.isMaximized().catch(() => false)) {
            await win.unmaximize();
          }
          await win.setResizable(false);
          await win.setSize(new LogicalSize(480, 640));
          await win.center();
        } catch (e) {
          console.warn("splash resize failed:", e);
        }
      }

      // FASE 0 — versión + chequeo de update de la app. Esto va
      // primero porque si hay update y el usuario acepta, instalar
      // mata el proceso y no tiene sentido cargar nada más.
      if (isTauri) {
        try {
          const { getVersion } = await import("@tauri-apps/api/app");
          const v = await getVersion();
          if (!cancelled) setAppVersion(v);
        } catch (e) {
          console.warn("getVersion failed:", e);
        }
      }
      await wrap(0, async () => {
        const u = await api.checkForUpdate().catch(() => null);
        if (cancelled) return;
        if (!u) return; // estamos al día
        // Mostrar el banner y bloquear hasta que el usuario decida.
        setAppUpdate(u);
        await new Promise<void>((resolve) => {
          setUpdateDecision({ resolve });
        });
        // Si el usuario eligió "Instalar", el effect que escucha
        // `updateInstalling` ya disparó `api.installUpdate` y la app
        // se cerrará sola. Aquí simplemente esperamos un buen rato
        // para que NO se vea la UI antes del exit.
        if (useInstallingFlagRef.current) {
          await new Promise<void>((r) => setTimeout(r, 60_000));
        }
      });

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
      const sourcesTask = wrap(1, () => sourcesPromise);

      const phase1 = Promise.all([
        sourcesTask,
        wrap(2, () => bootstrapSettings()),
        wrap(3, () => bootstrapSimBrief()),
        wrap(4, () => bootstrapFlightLog()),
        wrap(5, () => bootstrapDownloads()),
        // (v1.1.4) Hidratamos la lista de perfiles GSX locales sin
        // bloquear el splash — corre en paralelo con el resto. Sin
        // marca de tarea propia, fail silently si no hay carpeta.
        useGsxLocalStore.getState().refresh(),
      ]);

      // FASE 2 — scan del FS. Independiente de la red. Limitamos
      // a 8 segundos máx para no bloquear el splash si la carpeta
      // Community es muy grande; el scan completo continúa en
      // background si se trunca.
      const phase2 = wrap(6, () =>
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

      await Promise.allSettled([phase1, phase2]);

      // FASE 3 — refresh de updates contra catálogos + pre-load
      // de catálogos. Antes corrían en background tras dismiss;
      // ahora forman parte del splash para que cuando el usuario
      // vea la UI todo esté listo (notificaciones populadas +
      // pestaña Buscar con resultados visibles). Cap 12s para no
      // bloquear infinitamente si una fuente está caída.
      await wrap(7, () =>
        Promise.race([
          refreshUpdatesActive(),
          new Promise<void>((resolve) =>
            setTimeout(() => {
              console.warn("refresh updates hit 12s timeout");
              resolve();
            }, 12000),
          ),
        ]),
      );

      await wrap(8, async () => {
        const srcs = await sourcesPromise;
        if (!srcs || cancelled) return;
        await Promise.race([
          Promise.all(srcs.map((src) => preloadCatalog(src.id))),
          new Promise<void>((resolve) =>
            setTimeout(() => {
              console.warn("preload catalogs hit 10s timeout");
              resolve();
            }, 10000),
          ),
        ]);
      });

      if (!cancelled) {
        await new Promise((r) => setTimeout(r, 350));
        if (cancelled) return;
        if (isTauri) {
          try {
            const win = getCurrentWindow();
            // Modo "app normal": NO usamos title bar nativo — pintamos
            // un titlebar custom (`<TitleBar />`) con el mismo theme
            // dark que la app. Mantenemos `decorations: false` y
            // sólo agrandamos + centramos la ventana.
            //
            // NO llamamos `maximize()` al arrancar — el usuario
            // reportó que la ventana arrancaba maximizada y no se
            // podía mover de monitor (Windows bloquea drag en
            // maximize). Le dejamos un tamaño cómodo + centrada y
            // el usuario decide si maximizar via el botón del
            // titlebar.
            await win.setResizable(true);
            await win.setMinSize(new LogicalSize(960, 640));
            await win.setSize(new LogicalSize(1280, 820));
            await win.center();
            try {
              await win.setFocus();
            } catch {
              /* ignore */
            }
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

    // (v1.1.4) Listener para el botón "Importar..." de Settings —
    // abre el modal de importación de inventario con la ruta del
    // archivo seleccionado.
    const onImportInventory = (e: Event) => {
      const detail = (e as CustomEvent).detail as { path?: string };
      if (detail?.path) setImportInventoryPath(detail.path);
    };
    window.addEventListener(
      "msfs-addons:import-inventory" as never,
      onImportInventory as never,
    );

    return () => {
      cancelled = true;
      window.removeEventListener("focus", onFocus);
      window.removeEventListener(
        "msfs-addons:show-tour" as never,
        onShowTour as never,
      );
      window.removeEventListener(
        "msfs-addons:import-inventory" as never,
        onImportInventory as never,
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
    return (
      <SplashScreen
        tasks={splashTasks}
        appVersion={appVersion}
        appUpdate={appUpdate}
        installing={updateInstalling}
        updateProgress={updateProgress}
        onInstallUpdate={handleInstallUpdate}
        onSkipUpdate={handleSkipUpdate}
      />
    );
  }

  return (
    <div className="app-shell min-h-screen bg-gradient-to-b from-slate-950 via-slate-950 to-slate-900">
      {isTauri && <TitleBar />}
      {/* (v2.2.0) Banner de update visible siempre que haya una nueva
          versión y el usuario no la haya descartado. Se renderiza
          ARRIBA del header de la app, debajo del titlebar. */}
      <UpdateBanner />
      {!isTauri && (
        <div className="flex items-center justify-center gap-2 border-b border-amber-500/20 bg-amber-500/10 px-4 py-1.5 text-xs text-amber-200">
          <FlaskConical className="h-3.5 w-3.5" />
          {/* (v3.4.0) Texto i18n con un placeholder `{cmd}` que separamos
              al render para inyectar el <code> con estilo Tailwind. Más
              legible que concatenación + dangerouslySetInnerHTML. */}
          <span>
            {t("demo.title", { cmd: "__CMD__" })
              .split("__CMD__")
              .flatMap((chunk, i, arr) =>
                i < arr.length - 1
                  ? [
                      <span key={`t${i}`}>{chunk}</span>,
                      <code
                        key={`c${i}`}
                        className="rounded bg-amber-500/20 px-1 font-mono"
                      >
                        npm run tauri dev
                      </code>,
                    ]
                  : [<span key={`t${i}`}>{chunk}</span>],
              )}
          </span>
        </div>
      )}

      <header className="app-shell-header sticky top-9 z-10 border-b border-slate-800/80 bg-slate-950/80 backdrop-blur">
        {/* Header truly fluid — sin max-w. Se ajusta al ancho de la
            ventana con padding generoso a los lados. */}
        <div className="flex w-full items-center justify-between px-6 py-3">
          <div className="flex items-center gap-2 no-select">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-brand-500/15 ring-1 ring-brand-500/30">
              <Plane className="h-5 w-5 text-brand-300" />
            </div>
            <div>
              <h1 className="text-sm font-semibold tracking-wide text-slate-100">
                SimFleet
              </h1>
              <p className="text-[11px] text-slate-500">
                {t("header.tagline")}
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
              title={t("header.settings.tooltip")}
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
            label={t("nav.dashboard")}
            tourId="nav-dashboard"
          />
          <ViewTab
            active={view === "search"}
            onClick={() => setView("search")}
            icon={<ListChecks className="h-4 w-4" />}
            label={t("nav.search")}
            tourId="nav-search"
          />
          <ViewTab
            active={view === "map"}
            onClick={() => setView("map")}
            icon={<Globe2 className="h-4 w-4" />}
            label={t("nav.map")}
            tourId="nav-map"
          />
          <ViewTab
            active={view === "addons"}
            onClick={() => setView("addons")}
            icon={<Boxes className="h-4 w-4" />}
            label={t("nav.addons")}
            tourId="nav-addons"
          />
          <ViewTab
            active={view === "flightbook"}
            onClick={() => setView("flightbook")}
            icon={<Plane className="h-4 w-4" />}
            label={t("nav.flightbook")}
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
                  ? t("search.placeholder.simplaza")
                  : t("search.placeholder.scenery")
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
                  {t("common.previous")}
                </button>
                <span className="text-xs text-slate-500">
                  {t("common.page")} {browsePage}
                </span>
                <button
                  onClick={() => loadBrowsePage(browsePage + 1)}
                  disabled={status === "loading" || !browseHasMore}
                  className="rounded-md border border-slate-800 bg-slate-900/60 px-3 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 hover:text-slate-100 disabled:opacity-40"
                >
                  {t("common.next")}
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
      {tourOpen && (
        <OnboardingTour
          onClose={() => setTourOpen(false)}
          onSettingsOpenChange={setSettingsOpen}
        />
      )}
      {importInventoryPath && (
        <ImportInventoryModal
          path={importInventoryPath}
          onClose={() => setImportInventoryPath(null)}
        />
      )}
      <DragDropOverlay />
      <UpdateWizard />
      <Toaster />
      <SimBriefConfirmModal />
      <ReplayBanner />
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
