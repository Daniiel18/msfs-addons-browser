import { useEffect, useRef, useState } from "react";
import { FlaskConical, Search } from "lucide-react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api, isTauri } from "./lib/tauri";
import { setActiveLocale, t } from "./lib/i18n";
import { useAppStore } from "./stores/useAppStore";
import { useDownloadsStore } from "./stores/useDownloadsStore";
import { useCommunityStore } from "./stores/useCommunityStore";
import { useSimBriefStore } from "./stores/useSimBriefStore";
import { useFlightLogStore } from "./stores/useFlightLogStore";
import { useSettingsStore } from "./stores/useSettingsStore";
import { useGsxUpdateStore } from "./stores/useGsxUpdateStore";
import { useAiracUpdateStore } from "./stores/useAiracUpdateStore";
import { useGsxLocalStore } from "./stores/useGsxLocalStore";
import { useFlightsimUpdateStore } from "./stores/useFlightsimUpdateStore";
import { SourceToggle } from "./components/SourceToggle";
import { FlightsimEmbed, FlightsimReloadButton } from "./components/FlightsimEmbed";
import { initTechLogSync } from "./stores/useTechLogStore";
import { initRecorderNotices } from "./lib/recorderNotices";
import { SearchBar } from "./components/SearchBar";
import { ResultsList } from "./components/ResultsList";
import { DownloadsButton } from "./components/DownloadsButton";
import { PilotBadge } from "./components/PilotBadge";
import { DownloadsPanel } from "./components/DownloadsPanel";
import { NotificationsBell } from "./components/NotificationsBell";
import { MapView } from "./components/MapView";
import { AddonsView } from "./components/AddonsView";
import { DashboardView } from "./components/DashboardView";
import { FlightBookView } from "./components/FlightBookView";
import { HangarView } from "./components/HangarView";
import { EconomyView } from "./components/EconomyView";
import { TitleBar } from "./components/TitleBar";
import { Sidebar } from "./components/Sidebar";
import { LiveUpdateBadge } from "./components/LiveUpdateBadge";
import { DragDropOverlay } from "./components/DragDropOverlay";
import { UpdateWizard } from "./components/UpdateWizard";
import { PendingUpdatesModal } from "./components/PendingUpdatesModal";
import { UpdateBanner } from "./components/UpdateBanner";
import { Toaster } from "./components/Toaster";
import { LiveVsManager } from "./components/LiveVsManager";
import { GsxUpdateGate } from "./components/GsxUpdateGate";
import { GsxOfferModal } from "./components/GsxOfferModal";
import { useGsxOfferStore } from "./stores/useGsxOfferStore";
import {
  WhatsNewModal,
  isWhatsNewPending,
  WHATS_NEW_CLOSED_EVENT,
} from "./components/WhatsNewModal";
import { DownloadRecoveryModal } from "./components/DownloadRecoveryModal";
import { SimBriefConfirmModal } from "./components/SimBriefConfirmModal";
import { IncompleteFlightModal } from "./components/IncompleteFlightModal";
import { ReplayBanner } from "./components/ReplayBanner";
import { GsxSearchSummary } from "./components/GsxSearchSummary";
import { SettingsView } from "./components/SettingsModal";
import { CommandPalette } from "./components/CommandPalette";
import { PreflightHost } from "./components/PreflightHost";
import { SplashScreen, type SplashTask } from "./components/SplashScreen";
import type { UpdateInfo } from "./lib/types";
import { FlyingNowBadge } from "./components/FlyingNowBadge";
import { GsxRemoteBadge } from "./components/GsxRemoteBadge";
import { OnboardingTour } from "./components/OnboardingTour";
import { SimVersionModal } from "./components/SimVersionModal";
import { ImportInventoryModal } from "./components/ImportInventoryModal";
import { CrossLinkModal } from "./components/CrossLinkModal";
import type { CrossLinkOffer } from "./lib/types";
import { isTestEnv, lockDisabled } from "./lib/testEnv";

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
    setSources, setActiveSource, setQuery, setResults, setStatus, setError,
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

  // (v6.1) Bloquear F5 / Ctrl+R: recargar el webview reinicia toda la app
  // (pierde estado, re-bootstrapea). No es un navegador — la recarga manual
  // no debe existir. Captura en fase de captura para ganarle al webview.
  useEffect(() => {
    const blockReload = (e: KeyboardEvent) => {
      const isReload =
        e.key === "F5" ||
        ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r");
      if (isReload) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    window.addEventListener("keydown", blockReload, { capture: true });
    // (v6.2.3) Bloquear el menú contextual (clic derecho): da impresión de
    // navegador y deja a la vista "Recargar / Inspeccionar". No es una web.
    const blockContextMenu = (e: MouseEvent) => e.preventDefault();
    window.addEventListener("contextmenu", blockContextMenu, { capture: true });
    return () => {
      window.removeEventListener("keydown", blockReload, { capture: true });
      window.removeEventListener("contextmenu", blockContextMenu, {
        capture: true,
      });
    };
  }, []);

  // (v6.2.4) Chequeo de updates de GSX: una vez al arrancar y luego cada hora.
  // El resultado lo comparten Notificaciones y Dashboard vía el store.
  useEffect(() => {
    const checkGsx = useGsxUpdateStore.getState().check;
    // (v6.2.8) AIRAC: el ciclo es de calendario fijo (28 días). (v6.2.9) Sin
    // polling horario, pedido del usuario. (v7.4.2) Pero chequear SÓLO al
    // arrancar fallaba con la app abierta días en la bandeja: el aviso salía
    // el día que se reiniciaba, no el día efectivo. Ahora (todo local, sin
    // red): al arrancar, al volver a la ventana (p.ej. tras actualizar en
    // Navigraph Hub) y al cruzar la medianoche local.
    const checkAirac = useAiracUpdateStore.getState().check;
    void checkAirac();
    let midnightTimer = 0;
    const scheduleMidnight = () => {
      const now = new Date();
      const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 0, 0, 30);
      midnightTimer = window.setTimeout(() => {
        void checkAirac();
        scheduleMidnight();
      }, next.getTime() - now.getTime());
    };
    scheduleMidnight();
    // GSX sí se re-chequea: su release no sigue calendario.
    void checkGsx();
    const id = window.setInterval(() => void checkGsx(), 60 * 60 * 1000);
    const onFocus = () => {
      void checkGsx();
      void checkAirac();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(id);
      window.clearTimeout(midnightTimer);
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  const bootstrapSettings = useSettingsStore((s) => s.bootstrap);
  const settingsLoaded = useSettingsStore((s) => s.loaded);
  const simVersion = useSettingsStore((s) => s.settings.simVersion);
  // (v6.2.38) Fuentes visibles según la versión de MSFS. (v7.4.2) Simplaza
  // vuelve a MSFS2024 igual que SceneryAddons (el backend etiqueta/filtra
  // cada post según el sim elegido). Skybound sólo aplicaría a 2024, pero hoy
  // el backend ni lo lista (caído desde el 10/10).
  // (19/10) flightsim.to es la 3ª opción del switch de Explore: no es un
  // catálogo con búsqueda propia, sino su web DENTRO de la app. Persistido.
  const [fstoMode, setFstoModeState] = useState(() => {
    try {
      return localStorage.getItem("simfleet.explore.fsto") === "1";
    } catch {
      return false;
    }
  });
  // (25/10) Una búsqueda pedida desde fuera (campanita, modal de updates,
  // Ctrl+K) se ve aunque la última pestaña fuera flightsim.to.
  const searchRequest = useAppStore((s) => s.searchRequest);
  useEffect(() => {
    if (searchRequest > 0) setFstoMode(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchRequest]);
  // (25/10) …y una update de flightsim.to abre su pestaña de Explorar.
  const fstoTabRequest = useAppStore((s) => s.fstoTabRequest);
  useEffect(() => {
    if (fstoTabRequest > 0) setFstoMode(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fstoTabRequest]);
  const setFstoMode = (on: boolean) => {
    setFstoModeState(on);
    try {
      if (on) localStorage.setItem("simfleet.explore.fsto", "1");
      else localStorage.removeItem("simfleet.explore.fsto");
    } catch {
      // localStorage no disponible — el estado en memoria sigue ok.
    }
  };

  const visibleSources = sources.filter((s) => {
    if (s.id === "skybound") return simVersion === "msfs2024";
    return true;
  });
  // Si la fuente activa deja de estar visible (cambio de versión), saltamos
  // a la primera visible para no quedar en una pestaña vacía.
  useEffect(() => {
    if (
      visibleSources.length > 0 &&
      !visibleSources.some((s) => s.id === activeSourceId)
    ) {
      setActiveSource(visibleSources[0].id);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [simVersion, sources.length]);
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
        // (v4.26.0) El backend baja imágenes de Wikipedia para
        // aeropuertos sin thumbnail tras cada scan — al terminar
        // invalidamos el cache para que cards/popovers refresquen
        // sin reiniciar la app.
        const { clearThumbnailCache } = await import("./lib/thumbnails");
        const u3 = await api.onThumbsUpdated(() => {
          if (!cancelled) clearThumbnailCache();
        });
        unsubs.push(u3);
        // (v6 #1) Oferta de cross-link 2020/2024 tras instalar doble-compat.
        const u4 = await api.onCrossLinkOffer((offer) => {
          if (!cancelled) setCrossLinkOffer(offer);
        });
        unsubs.push(u4);
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
  // (19/10) Etapa 2: sincronía con Fenix/PMDG TechLog (matrículas gestionadas
  // + escucha de cambios en vivo) una vez cargada la app.
  useEffect(() => {
    if (ready) {
      initTechLogSync();
      initRecorderNotices();
    }
  }, [ready]);
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

  // (19/10) BLOQUEO DE INICIO: la app no se abre hasta verificar al usuario
  // de Windows (Windows Hello o su contraseña de Windows). La carga sigue en
  // segundo plano; al terminar, el bootstrap espera aquí al desbloqueo.
  // En el modo demo (navegador) no hay bloqueo. (25/10) Ni en el entorno de
  // pruebas (`tauri dev`); en producción, siempre.
  const [unlocked, setUnlocked] = useState(!isTauri || lockDisabled());
  const unlockedRef = useRef(!isTauri || lockDisabled());
  const unlockWaiterRef = useRef<(() => void) | null>(null);
  const [lockInfo, setLockInfo] = useState<{ helloAvailable: boolean; username: string } | null>(null);
  const [lockBusy, setLockBusy] = useState(false);
  const [lockError, setLockError] = useState<string | null>(null);
  const [bootWaitingUnlock, setBootWaitingUnlock] = useState(false);
  const helloAutoTriedRef = useRef(false);
  useEffect(() => {
    if (!isTauri) return;
    // (v8.3) Si la página se recargó con flightsim.to abierto, su webview
    // nativo sigue vivo y quedaría flotando sobre el bloqueo: fuera.
    void api.fstoEmbedHide().catch(() => {});
    api
      .lockInfo()
      .then(setLockInfo)
      .catch(() => setLockInfo({ helloAvailable: false, username: "" }));
  }, []);
  const markUnlocked = () => {
    unlockedRef.current = true;
    setUnlocked(true);
    setLockError(null);
    unlockWaiterRef.current?.();
    unlockWaiterRef.current = null;
  };
  const unlockWithHello = async () => {
    setLockBusy(true);
    setLockError(null);
    try {
      const r = await api.lockVerifyHello(t("lock.hello_message"));
      if (r === "verified") markUnlocked();
      else if (r === "unavailable") {
        setLockInfo((i) => (i ? { ...i, helloAvailable: false } : i));
        setLockError(t("lock.err.hello_unavailable"));
      } else if (r === "canceled") setLockError(t("lock.err.canceled"));
      else if (r === "retriesExhausted") setLockError(t("lock.err.retries"));
      else setLockError(t("lock.err.failed"));
    } catch (e) {
      setLockError(String(e));
    } finally {
      setLockBusy(false);
    }
  };
  const unlockWithPassword = async (password: string) => {
    setLockBusy(true);
    setLockError(null);
    try {
      if (await api.lockVerifyPassword(password)) markUnlocked();
      else setLockError(t("lock.err.wrong_password"));
    } catch (e) {
      setLockError(String(e));
    } finally {
      setLockBusy(false);
    }
  };
  // Al terminar la carga, si sigue bloqueada, lanzamos Windows Hello una vez.
  useEffect(() => {
    if (!bootWaitingUnlock || unlocked || !lockInfo?.helloAvailable) return;
    if (helloAutoTriedRef.current) return;
    helloAutoTriedRef.current = true;
    void unlockWithHello();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bootWaitingUnlock, unlocked, lockInfo]);
  // Promesa que resuelve cuando el usuario decide qué hacer con la
  // update (instalar o saltar). El bootstrap espera por esto.
  const [updateDecision, setUpdateDecision] = useState<{
    resolve: () => void;
  } | null>(null);

  // (v6 · v7.4.2) ENTORNO DE PRUEBAS: 2ª instancia junto a la principal con
  // datos, WebView2, Drive, Supabase y autoarranque propios.
  // Indicadores: banner rojo + badge en el header + TÍTULO DE VENTANA (para
  // distinguirla en la taskbar/Alt-Tab de la instancia principal). El flag
  // llega inyectado por el backend → se conoce desde el primer render.
  const [safeMode, setSafeMode] = useState(isTestEnv);
  const [watcherActive, setWatcherActive] = useState(false);
  const [testDataDir, setTestDataDir] = useState<string | null>(null);
  useEffect(() => {
    if (!isTauri) return;
    api
      .isSafeMode()
      .then((sm) => {
        setSafeMode(sm);
        // (El título de la ventana lo pone el backend al arrancar.)
        if (sm) api.testEnvDataDir().then(setTestDataDir).catch(() => {});
      })
      .catch(() => {});
    api
      .isWatcherActive()
      .then(setWatcherActive)
      .catch(() => {});
  }, []);

  // (Ajustes 19/10) Ajustes es una vista (useAppStore.view === "settings").
  const openSettings = () => useAppStore.getState().setView("settings");
  const [tourOpen, setTourOpen] = useState(false);
  // (v6 — #4) Modal de novedades "What's New" anti-skip. Se abre tras `ready`:
  // en la instancia principal sólo cuando la app SE ACTUALIZA (la versión
  // guardada en localStorage difiere de la actual), una vez por versión; en
  // MODO SEGURO siempre. No coexiste con el tour de bienvenida.
  // (v6 — #1) Oferta de cross-link 2020/2024 tras instalar un escenario
  // doble-compat. El backend la emite por `cross-link://offer`.
  const [crossLinkOffer, setCrossLinkOffer] = useState<CrossLinkOffer | null>(
    null,
  );
  // (v5.2.3) Gate de versión de MSFS DURANTE el splash. Si es el primer
  // arranque (simVersion vacío), el bootstrap muestra el chooser ANTES de
  // escanear Community y espera por esta promesa para continuar con la
  // versión ya aplicada en el backend.
  const [simVersionChoice, setSimVersionChoice] = useState<{
    resolve: () => void;
  } | null>(null);
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
    // (fix) Persistir la versión saltada para que el modal BLOQUEANTE del
    // splash NO reaparezca en cada arranque. Antes sólo se descartaba por
    // sesión → volvía a salir siempre (el banner in-app `UpdateBanner`
    // sigue ofreciendo el update dentro de la app).
    try {
      if (appUpdate?.latestVersion) {
        localStorage.setItem("sf_splash_update_skipped", appUpdate.latestVersion);
      }
    } catch {
      /* ignore */
    }
    updateDecision?.resolve();
    setUpdateDecision(null);
  };

  // (v2.2.0 → v5.3.3) Auto-refresh de SimBrief mientras MSFS corre. El
  // watcher reporta `simRunning` cuando detecta el proceso o el handshake de
  // SimConnect. Mientras esté true + haya pilotId, pollamos cada 30s.
  //
  // (v5.3.3) QUITADAS las condiciones de parada `planSyncedWithAirport` y
  // `atDepartureGate`: detenían el polling ANTES de que el usuario generara
  // su OFP (al posicionarse en el gate), y con varios OFP del mismo origen
  // en caché `planSyncedWithAirport` daba falsos positivos. Resultado: el
  // OFP recién generado NO se capturaba y había que refrescar a mano (bug
  // reportado). Ahora se pollea de forma constante mientras el sim corre,
  // así el OFP nuevo entra en ≤30s — esté el avión en el gate o ya volando.
  useEffect(() => {
    if (!ready) return;
    if (!simRunning) return;
    if (!simBriefPilotId) return;
    let cancelled = false;
    const tick = () => {
      if (cancelled) return;
      void refreshSimBrief().catch((e) =>
        console.warn("auto-refresh simbrief:", e),
      );
    };
    // Trigger inicial.
    tick();
    const interval = setInterval(tick, 30_000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [ready, simRunning, simBriefPilotId, refreshSimBrief]);

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
        // (fix) Si el usuario ya SALTÓ esta versión en el splash, no volver
        // a bloquear en cada arranque — el banner in-app sigue disponible.
        try {
          if (localStorage.getItem("sf_splash_update_skipped") === u.latestVersion) {
            return;
          }
        } catch {
          /* ignore */
        }
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

      // (v5.2.3) Settings PRIMERO + gate de versión de MSFS. El scan de
      // Community y el filtro del catálogo dependen de la versión elegida,
      // así que en el PRIMER arranque hay que pedirla AQUÍ (en el splash)
      // antes de escanear — no después de abrir la app, que era el bug
      // reportado. El resto de bootstraps de phase1 NO dependen de la
      // versión, así que siguen corriendo en paralelo mientras esperamos.
      const settingsTask = wrap(2, () => bootstrapSettings());
      const otherPhase1 = Promise.all([
        sourcesTask,
        wrap(3, () => bootstrapSimBrief()),
        wrap(4, () => bootstrapFlightLog()),
        wrap(5, () => bootstrapDownloads()),
        // (v1.1.4) Hidratamos la lista de perfiles GSX locales sin
        // bloquear el splash — corre en paralelo con el resto. Sin
        // marca de tarea propia, fail silently si no hay carpeta.
        // (v6.2.44) Tras conocer los instalados, chequeamos updates en
        // background (pega a flightsim.to por ICAO) sin bloquear nada.
        useGsxLocalStore
          .getState()
          .refresh()
          .then(() => useGsxLocalStore.getState().refreshUpdates()),
        // (v6.2.60) Updates de addons descargados de flightsim.to (baseline por
        // file_id, fiable). En background, sin bloquear el splash.
        useFlightsimUpdateStore.getState().refreshUpdates(),
      ]);

      // (v6.2.64) Oferta de perfiles GSX al instalar un escenario nuevo con
      // ICAO. Suscribe el diff del listado de Community (idempotente).
      useGsxOfferStore.getState().start();

      // (v5.3.4) Gate de versión de MSFS según lo INSTALADO:
      //   · 2020 Y 2024 instalados → preguntar en el splash CADA arranque
      //     (el usuario elige con qué versión trabajar esta sesión).
      //   · Solo una instalada → auto-seleccionarla, sin preguntar.
      //   · Ninguna detectada + sin preferencia previa → preguntar (1er run).
      // El backend fija su atómico al persistir (`set_app_setting` →
      // `sim::set_from_str`), así el scan que sigue usa la versión correcta.
      await settingsTask;
      if (isTauri && !cancelled) {
        const sims = await api
          .getInstalledSims()
          .catch(() => ({ has2020: false, has2024: false }));
        const ver = useSettingsStore.getState().settings.simVersion;
        if (sims.has2020 && sims.has2024) {
          // (v6 fix) Si el usuario acaba de cambiar la versión desde Ajustes y
          // la app se recargó ("restart now"), NO volver a preguntar — la
          // elección ya está persistida. El flag es de un solo uso.
          let skipOnce = false;
          try {
            skipOnce =
              localStorage.getItem("simfleet:skip-version-prompt-once") !==
              null;
            if (skipOnce)
              localStorage.removeItem("simfleet:skip-version-prompt-once");
          } catch {
            /* ignore */
          }
          if (!skipOnce) {
            await new Promise<void>((resolve) => {
              setSimVersionChoice({ resolve });
            });
          }
        } else if (sims.has2020 && !sims.has2024) {
          if (ver !== "msfs2020") {
            await useSettingsStore.getState().setSimVersion("msfs2020");
          }
        } else if (sims.has2024 && !sims.has2020) {
          if (ver !== "msfs2024") {
            await useSettingsStore.getState().setSimVersion("msfs2024");
          }
        } else if (!ver) {
          await new Promise<void>((resolve) => {
            setSimVersionChoice({ resolve });
          });
        }
      }
      if (cancelled) return;

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

      await Promise.allSettled([otherPhase1, phase2]);

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
        // (19/10) Bloqueo de inicio: esperamos a que el usuario se verifique
        // ANTES de agrandar la ventana y mostrar la app.
        if (!unlockedRef.current) {
          setBootWaitingUnlock(true);
          await new Promise<void>((resolve) => {
            unlockWaiterRef.current = resolve;
          });
          if (cancelled) return;
        }
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
          const firstRun = !skipped && !done;

          // ── What's New ──────────────────────────────────────────────
          // · Instancia de pruebas (MODO SEGURO): SIEMPRE se muestra (para
          //   poder revisarlo); al cerrar NO persiste, así reaparece cada vez.
          // · Instancia principal: SÓLO cuando la app SE ACTUALIZA (la versión
          //   cambia), una sola vez por versión. Sin línea base (1ª instalación
          //   o 1ª vez con esta feature) fijamos la versión SIN mostrar nada,
          //   para no enseñar novedades retroactivas.
          // Sale en MODO SEGURO siempre, y en la instancia principal cada vez
          // que la versión cambia (incluida la 1ª vez sin línea base — el
          // usuario lo pidió: "debe salir cada vez que se actualiza"). Al
          // cerrarlo se marca la versión vista, así no reaparece hasta el
          // siguiente bump.
          // (v6.2.55) What's New ELIMINADO por completo (pedido del usuario).

          // Tour de bienvenida sólo para usuarios nuevos. Si el modal BLOQUEANTE
          // de novedades va a salir (instalación nueva), el tour ESPERA a que se
          // cierre — si no, su backdrop taparía el botón «Continuar» del modal y
          // dejaría la app en soft-lock.
          if (firstRun) {
            if (isWhatsNewPending()) {
              window.addEventListener(
                WHATS_NEW_CLOSED_EVENT,
                () => setTourOpen(true),
                { once: true },
              );
            } else {
              setTourOpen(true);
            }
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

  // (19/10) Sin desbloquear no se muestra la app, ni siquiera si el
  // bootstrap falló (su catch hace setReady(true)).
  if (!ready || !unlocked) {
    return (
      <>
        <SplashScreen
          tasks={splashTasks}
          appVersion={appVersion}
          appUpdate={appUpdate}
          installing={updateInstalling}
          updateProgress={updateProgress}
          onInstallUpdate={handleInstallUpdate}
          onSkipUpdate={handleSkipUpdate}
          lock={{
            required: !unlocked,
            helloAvailable: lockInfo ? lockInfo.helloAvailable : null,
            username: lockInfo?.username ?? "",
            busy: lockBusy,
            error: lockError,
            waiting: bootWaitingUnlock,
            onHello: () => void unlockWithHello(),
            onPassword: (pw) => void unlockWithPassword(pw),
          }}
        />
        {/* (v5.2.3) Elección de versión de MSFS en el splash, ANTES del
            scan de Community. Al elegir, fija el atómico del backend y
            resuelve el gate para que el bootstrap continúe. */}
        {simVersionChoice && (
          <SimVersionModal
            onChosen={() => {
              simVersionChoice.resolve();
              setSimVersionChoice(null);
            }}
          />
        )}
      </>
    );
  }

  return (
    /* (v4.29.0) Shell de altura-viewport + overflow hidden. El body ya
       no scrollea — cada vista tiene su propio scroll.
       (Rediseño F1) Menú lateral a la izquierda y, a su derecha, una
       columna con la barra superior única + banners + contenido. */
    <div className="app-shell flex h-screen overflow-hidden bg-sf-bg text-sf-text">
      <Sidebar safeMode={safeMode} />

      <div className="flex min-w-0 flex-1 flex-col">
        <TitleBar
          leading={
            <button
              onClick={() => window.dispatchEvent(new Event("simfleet:palette"))}
              title={t("palette.tooltip")}
              aria-label={t("palette.tooltip")}
              className="flex h-8 w-8 min-w-0 shrink-0 items-center justify-center gap-2 rounded-lg border border-sf-border bg-sf-surface text-sf-muted transition-colors hover:border-brand-500/40 hover:text-sf-text lg:w-72 lg:shrink lg:justify-start lg:px-3 xl:w-96"
            >
              <Search className="h-4 w-4 shrink-0" />
              <span className="hidden truncate text-[13px] lg:inline">
                {t("topbar.search_placeholder")}
              </span>
              <kbd className="ml-auto hidden shrink-0 rounded border border-sf-border bg-sf-bg px-1.5 py-0.5 text-[10px] font-medium text-sf-muted lg:inline">
                Ctrl K
              </kbd>
            </button>
          }
          actions={
            <>
              {/* Badges de estado: si falta ancho, se recortan ellos antes
                  que los accesos o los controles de ventana. */}
              <div className="flex min-w-0 shrink items-center gap-2 overflow-hidden">
                <FlyingNowBadge />
                <GsxRemoteBadge />
                <LiveUpdateBadge />
              </div>
              <span data-tour-id="header-notifications" className="shrink-0">
                <NotificationsBell />
              </span>
              <span className="shrink-0">
                <DownloadsButton />
              </span>
              <span className="shrink-0">
                <PilotBadge />
              </span>
            </>
          }
        />
        {/* (v2.2.0) Banner de update visible siempre que haya una nueva
            versión y el usuario no la haya descartado. Debajo de la barra
            superior. */}
        <UpdateBanner />
        {safeMode && (
          <div className="flex items-center justify-center gap-2 border-b border-rose-500/30 bg-rose-500/15 px-4 py-1.5 text-xs font-semibold text-rose-200">
            🧪{" "}
            {t(watcherActive ? "safeMode.bannerActive" : "safeMode.bannerIdle", {
              dir: testDataDir ?? "…",
            })}
          </div>
        )}
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

        <main
          className={`min-h-0 flex-1 w-full overflow-hidden px-6 pb-5 pt-5 ${
            view === "search" ? "mx-auto max-w-[1600px]" : ""
          }`}
        >
          {/* (v4.29.0) Dashboard y Search pueden tener contenido alto;
              envolvemos en wrapper scrolleable INTERNO. Map, Addons y
              FlightBook ya tienen sus propios layouts h-full. */}
          {view === "dashboard" && (
            <div className="h-full overflow-y-auto">
              <DashboardView />
            </div>
          )}

          {view === "search" && (
            <div
              className={
                fstoMode ? "flex h-full min-h-0 flex-col" : "h-full overflow-y-auto"
              }
            >
              <div
                className={
                  fstoMode ? "flex min-h-0 flex-1 flex-col gap-4" : "space-y-4 pb-2"
                }
              >
                {/* (Rediseño F3) Cabecera de Explorar: el selector de
                    catálogo y flightsim.to viven aquí, junto al buscador
                    (antes estaban en la barra superior sólo en esta vista). */}
                <header className="flex flex-wrap items-end justify-between gap-3">
                  <div className="min-w-0">
                    <h1 className="text-xl font-semibold tracking-tight text-sf-text">
                      {t("explore.title")}
                    </h1>
                    <p className="mt-0.5 text-sm text-sf-muted">
                      {t("explore.subtitle")}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center gap-2">
                    {/* (v8.3) flightsim.to sin barra propia: sólo recargar. */}
                    {fstoMode && <FlightsimReloadButton />}
                    {/* (19/10) flightsim.to entra al switch como 3ª opción
                        (antes era un botón aparte que abría otra ventana). */}
                    <SourceToggle
                      sources={[
                        ...visibleSources,
                        { id: "flightsim", name: "flightsim.to", homeUrl: "https://flightsim.to" },
                      ]}
                      activeId={fstoMode ? "flightsim" : activeSourceId}
                      onChange={(id) => {
                        if (id === "flightsim") {
                          setFstoMode(true);
                        } else {
                          setFstoMode(false);
                          setActiveSource(id);
                        }
                      }}
                    />
                  </div>
                </header>
                {fstoMode ? (
                  <FlightsimEmbed />
                ) : (
                <>
                <SearchBar
                  value={query}
                  onChange={setQuery}
                  onSubmit={runSearch}
                  loading={status === "loading"}
                  placeholder={
                    activeSourceId === "simplaza"
                      ? t("search.placeholder.simplaza")
                      : activeSourceId === "aerohub"
                        ? t("search.placeholder.aerohub")
                        : t("search.placeholder.scenery")
                  }
                />
                {activeSourceId === "sceneryaddons" && (
                  <div className="flex justify-center empty:hidden">
                    <GsxSearchSummary query={query} />
                  </div>
                )}
                <ResultsList
                  results={results}
                  status={status}
                  error={error}
                  query={query}
                  browseMode={browseMode}
                />
                {browseMode === "browse" && results.length > 0 && (
                  <div className="flex items-center justify-center gap-2 pt-2">
                    <button
                      onClick={() => loadBrowsePage(Math.max(1, browsePage - 1))}
                      disabled={status === "loading" || browsePage <= 1}
                      className="rounded-lg border border-sf-border bg-sf-surface px-3 py-1.5 text-xs font-medium text-sf-text hover:border-brand-500/40 disabled:opacity-40"
                    >
                      {t("common.previous")}
                    </button>
                    <span className="px-2 text-xs tabular-nums text-sf-muted">
                      {t("common.page")} {browsePage}
                    </span>
                    <button
                      onClick={() => loadBrowsePage(browsePage + 1)}
                      disabled={status === "loading" || !browseHasMore}
                      className="rounded-lg border border-sf-border bg-sf-surface px-3 py-1.5 text-xs font-medium text-sf-text hover:border-brand-500/40 disabled:opacity-40"
                    >
                      {t("common.next")}
                    </button>
                  </div>
                )}
                </>
                )}
              </div>
            </div>
          )}
          {view === "map" && <MapView />}
          {view === "addons" && <AddonsView />}
          {view === "flightbook" && <FlightBookView />}
          {view === "hangar" && <HangarView />}
          {view === "economy" && <EconomyView />}
          {view === "settings" && <SettingsView />}
        </main>
      </div>

      <DownloadsPanel open={isPanelOpen} onClose={() => setPanelOpen(false)} />

      {/* (v6.2.27 / R4) Paleta de comandos global — Ctrl/Cmd+K. */}
      <CommandPalette onOpenSettings={openSettings} />
      {/* (v6.2.34) Pre-vuelo "¿Listo para volar?" — auto-alerta al OFP del día. */}
      <PreflightHost />
      {tourOpen && (
        <OnboardingTour
          onClose={() => setTourOpen(false)}
          onSettingsOpenChange={(o) => {
            const s = useAppStore.getState();
            if (o) s.setView("settings");
            else if (s.view === "settings") s.setView("dashboard");
          }}
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
      <PendingUpdatesModal />
      <Toaster />
      <LiveVsManager />
      <GsxUpdateGate />
      <GsxOfferModal />
      <WhatsNewModal />
      <DownloadRecoveryModal />
      <SimBriefConfirmModal />
      <IncompleteFlightModal />
      <ReplayBanner />
      {/* (v5.1.0) Elección de versión de MSFS al primer arranque. */}
      {settingsLoaded && simVersion === "" && <SimVersionModal />}
      {/* (v6 — #4) Novedades "What's New" anti-skip. Al cerrar, en la
          instancia principal persistimos la versión vista (una vez por
          actualización); en MODO SEGURO no, para que reaparezca siempre. */}
      {/* (v6.2.55) What's New ELIMINADO por completo (pedido del usuario). */}
      {/* (v6 — #1) Oferta de cross-link 2020/2024 tras instalar doble-compat. */}
      {crossLinkOffer && (
        <CrossLinkModal
          offer={crossLinkOffer}
          onClose={() => setCrossLinkOffer(null)}
        />
      )}
    </div>
  );
}
