import { useEffect, useRef, useState } from "react";
import {
  BarChart3,
  Boxes,
  FlaskConical,
  Globe2,
  ListChecks,
  Plane,
  Search,
  Settings,
  Wallet,
  Warehouse,
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
import { useGsxUpdateStore } from "./stores/useGsxUpdateStore";
import { useAiracUpdateStore } from "./stores/useAiracUpdateStore";
import { useGsxLocalStore } from "./stores/useGsxLocalStore";
import { SourceToggle } from "./components/SourceToggle";
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
import { DragDropOverlay } from "./components/DragDropOverlay";
import { UpdateWizard } from "./components/UpdateWizard";
import { UpdateBanner } from "./components/UpdateBanner";
import { Toaster } from "./components/Toaster";
import { LiveVsManager } from "./components/LiveVsManager";
import { GsxUpdateGate } from "./components/GsxUpdateGate";
import { SimBriefConfirmModal } from "./components/SimBriefConfirmModal";
import { IncompleteFlightModal } from "./components/IncompleteFlightModal";
import { ReplayBanner } from "./components/ReplayBanner";
import { GsxSearchSummary } from "./components/GsxSearchSummary";
import { SettingsModal } from "./components/SettingsModal";
import { CommandPalette } from "./components/CommandPalette";
import { SplashScreen, type SplashTask } from "./components/SplashScreen";
import type { UpdateInfo } from "./lib/types";
import { FlyingNowBadge } from "./components/FlyingNowBadge";
import { OnboardingTour } from "./components/OnboardingTour";
import { SimVersionModal } from "./components/SimVersionModal";
import { ImportInventoryModal } from "./components/ImportInventoryModal";
import {
  WhatsNewModal,
  buildWhatsNewSlides,
  isUpdateSinceLastSeen,
  markWhatsNewSeen,
} from "./components/WhatsNewModal";
import { CrossLinkModal } from "./components/CrossLinkModal";
import type { CrossLinkOffer } from "./lib/types";

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
    loadBrowsePage, openEconomy,
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
    // (v6.2.8) AIRAC: el ciclo es de calendario fijo (28 días) → basta validarlo
    // UNA vez al arrancar. (v6.2.9) Sin polling horario, pedido del usuario.
    void useAiracUpdateStore.getState().check();
    // GSX sí se re-chequea: su release no sigue calendario.
    void checkGsx();
    const id = window.setInterval(() => void checkGsx(), 60 * 60 * 1000);
    const onFocus = () => void checkGsx();
    window.addEventListener("focus", onFocus);
    return () => {
      window.clearInterval(id);
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  const bootstrapSettings = useSettingsStore((s) => s.bootstrap);
  const settingsLoaded = useSettingsStore((s) => s.loaded);
  const simVersion = useSettingsStore((s) => s.settings.simVersion);
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

  // (v6) Modo seguro: 2ª instancia de pruebas sin SimConnect/cloud.
  // Indicadores: banner rojo + badge en el header + TÍTULO DE VENTANA (para
  // distinguirla en la taskbar/Alt-Tab de la instancia principal).
  const [safeMode, setSafeMode] = useState(false);
  // (v6) En MODO SEGURO el watcher normalmente está apagado, pero se puede
  // forzar (SIMFLEET_ENABLE_WATCHER) para probar Best Landings. El banner lo
  // refleja para no confundir (SimConnect activo, nube siempre apagada).
  const [watcherActive, setWatcherActive] = useState(false);
  useEffect(() => {
    if (!isTauri) return;
    api
      .isSafeMode()
      .then((sm) => {
        setSafeMode(sm);
        if (sm) {
          getCurrentWindow()
            .setTitle("SimFleet — MODO SEGURO (pruebas)")
            .catch(() => {});
        }
      })
      .catch(() => {});
    api
      .isWatcherActive()
      .then(setWatcherActive)
      .catch(() => {});
  }, []);

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [tourOpen, setTourOpen] = useState(false);
  // (v6 — #4) Modal de novedades "What's New" anti-skip. Se abre tras `ready`:
  // en la instancia principal sólo cuando la app SE ACTUALIZA (la versión
  // guardada en localStorage difiere de la actual), una vez por versión; en
  // MODO SEGURO siempre. No coexiste con el tour de bienvenida.
  const [whatsNewOpen, setWhatsNewOpen] = useState(false);
  // (v6.1) Identidad del piloto (daniel/hector) para personalizar el slide
  // de Crew VS del What's New (el badge "TÚ" en la tarjeta correcta).
  const [pilotIdentity, setPilotIdentity] = useState<string | null>(null);
  useEffect(() => {
    api
      .pilotProfile()
      .then((p) => setPilotIdentity(p.identity))
      .catch(() => {});
  }, []);
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
    // (v6) Capturadas DENTRO del closure del bootstrap — las versiones de
    // `appVersion`/`safeMode` en el scope del render quedan "stale" aquí
    // (el effect corre una vez al montar). Las usamos para decidir el What's New.
    let resolvedVersion: string | null = null;
    let resolvedSafeMode = false;
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
          resolvedVersion = v;
          if (!cancelled) setAppVersion(v);
        } catch (e) {
          console.warn("getVersion failed:", e);
        }
        resolvedSafeMode = await api.isSafeMode().catch(() => false);
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
        useGsxLocalStore.getState().refresh(),
      ]);

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
          if (resolvedSafeMode || isUpdateSinceLastSeen(resolvedVersion)) {
            setWhatsNewOpen(true);
          }

          // Tour de bienvenida sólo para usuarios nuevos.
          if (firstRun) setTourOpen(true);
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
      <>
        <SplashScreen
          tasks={splashTasks}
          appVersion={appVersion}
          appUpdate={appUpdate}
          installing={updateInstalling}
          updateProgress={updateProgress}
          onInstallUpdate={handleInstallUpdate}
          onSkipUpdate={handleSkipUpdate}
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
    /* (v4.29.0) Shell de altura-viewport: flex column + overflow
       hidden. El body ya no scrollea — cada vista tiene su propio
       scroll. La rueda sobre el mapa ya no desplaza el header. */
    <div className="app-shell flex h-screen flex-col overflow-hidden bg-gradient-to-b from-slate-950 via-slate-950 to-slate-900">
      {isTauri && <TitleBar />}
      {/* (v2.2.0) Banner de update visible siempre que haya una nueva
          versión y el usuario no la haya descartado. Se renderiza
          ARRIBA del header de la app, debajo del titlebar. */}
      <UpdateBanner />
      {safeMode && (
        <div className="flex items-center justify-center gap-2 border-b border-rose-500/30 bg-rose-500/15 px-4 py-1.5 text-xs font-semibold text-rose-200">
          {watcherActive ? (
            <>
              🛡️ MODO SEGURO (pruebas) — SimConnect ACTIVO para probar Best
              Landings · la nube sigue DESACTIVADA. DB independiente de tu
              instancia principal.
            </>
          ) : (
            <>
              🛡️ MODO SEGURO — instancia de pruebas. SimConnect y la
              sincronización con la nube están DESACTIVADOS; no afecta el vuelo
              ni los datos de tu instancia principal.
            </>
          )}
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

      <header className="app-shell-header sticky top-9 z-40 border-b border-slate-800/80 bg-slate-950/80 backdrop-blur">
        {/* Header truly fluid — sin max-w. Se ajusta al ancho de la
            ventana con padding generoso a los lados. */}
        <div className="flex w-full items-center justify-between px-6 py-3">
          <div className="flex items-center gap-2 no-select">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-brand-500/15 ring-1 ring-brand-500/30">
              <Plane className="h-5 w-5 text-brand-300" />
            </div>
            <div>
              <h1 className="flex items-center gap-1.5 text-sm font-semibold tracking-wide text-slate-100">
                SimFleet
                {safeMode && (
                  <span className="rounded bg-rose-500/20 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider text-rose-300 ring-1 ring-rose-500/40">
                    Safe
                  </span>
                )}
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
              onClick={() => window.dispatchEvent(new Event("simfleet:palette"))}
              title={t("palette.tooltip")}
              className="hidden items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-900/60 px-2.5 py-2 text-slate-400 hover:border-brand-500/40 hover:text-slate-100 sm:inline-flex"
            >
              <Search className="h-4 w-4" />
              <kbd className="rounded border border-slate-700 bg-slate-950 px-1 py-0.5 text-[9px] font-medium text-slate-500">
                Ctrl K
              </kbd>
            </button>
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
            <PilotBadge />
          </div>
        </div>
      </header>

      <DownloadsPanel open={isPanelOpen} onClose={() => setPanelOpen(false)} />

      {/* (v4.27.0) Nav FUERA del main para que:
          1. Mantenga el MISMO ancho en todas las pantallas (antes en
             Search se encogía con `max-w-[1600px]`).
          2. Sea sticky en scroll — siempre accesible aunque la vista
             tenga contenido largo.
          El cap de Search se aplica solo a su contenido (más abajo). */}
      <nav className="sticky top-0 z-30 mx-6 mt-5 flex gap-1 rounded-xl bg-slate-900/80 p-1 ring-1 ring-slate-800 backdrop-blur">
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
          <ViewTab
            active={view === "hangar"}
            onClick={() => setView("hangar")}
            icon={<Warehouse className="h-4 w-4" />}
            label={t("nav.hangar")}
            tourId="nav-hangar"
          />
          <ViewTab
            active={view === "economy"}
            onClick={() => openEconomy(null)}
            icon={<Wallet className="h-4 w-4" />}
            label={t("nav.economy")}
            tourId="nav-economy"
          />
      </nav>

      <main
        className={`min-h-0 flex-1 w-full overflow-hidden px-6 pb-5 pt-4 ${
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
          <div className="h-full overflow-y-auto">
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
          </div>
        )}
        {view === "map" && <MapView />}
        {view === "addons" && <AddonsView />}
        {view === "flightbook" && <FlightBookView />}
        {view === "hangar" && <HangarView />}
        {view === "economy" && <EconomyView />}
      </main>

      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
      {/* (v6.2.27 / R4) Paleta de comandos global — Ctrl/Cmd+K. */}
      <CommandPalette onOpenSettings={() => setSettingsOpen(true)} />
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
      <LiveVsManager />
      <GsxUpdateGate />
      <SimBriefConfirmModal />
      <IncompleteFlightModal />
      <ReplayBanner />
      {/* (v5.1.0) Elección de versión de MSFS al primer arranque. */}
      {settingsLoaded && simVersion === "" && <SimVersionModal />}
      {/* (v6 — #4) Novedades "What's New" anti-skip. Al cerrar, en la
          instancia principal persistimos la versión vista (una vez por
          actualización); en MODO SEGURO no, para que reaparezca siempre. */}
      {whatsNewOpen && (
        <WhatsNewModal
          slides={buildWhatsNewSlides(pilotIdentity)}
          onClose={() => {
            setWhatsNewOpen(false);
            if (!safeMode && appVersion) markWhatsNewSeen(appVersion);
          }}
        />
      )}
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
