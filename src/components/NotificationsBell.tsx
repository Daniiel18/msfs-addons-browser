import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import {
  Bell,
  CheckCheck,
  Download,
  ExternalLink,
  FlaskConical,
  Map as MapIcon,
  RefreshCw,
  Sparkles,
  Truck,
  Navigation,
  Rocket,
  Swords,
  Wrench,
  Gauge,
  X,
} from "lucide-react";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useAppStore } from "../stores/useAppStore";
import {
  useGsxUpdateStore,
  gsxUpdateVisible,
} from "../stores/useGsxUpdateStore";
import {
  useAiracUpdateStore,
  airacUpdateVisible,
} from "../stores/useAiracUpdateStore";
import { useLiveVsStore } from "../stores/useLiveVsStore";
import { useFlightsimUpdateStore } from "../stores/useFlightsimUpdateStore";
import { useGsxOfferStore } from "../stores/useGsxOfferStore";
import { useMaintAlertsStore } from "../stores/useMaintAlertsStore";
import { useFpsNoticesStore } from "../stores/useFpsNoticesStore";
import { usePreflight } from "../lib/usePreflight";
import { usePreflightStore } from "../stores/usePreflightStore";
import { PerformanceModal } from "./PerformanceModal";
import type {
  AvailableUpdate,
  CommunityPackage,
  UpdateInfo,
} from "../lib/types";
import { api } from "../lib/tauri";
import { deriveUpdateSearchQuery } from "../lib/updateSearchQuery";
import { t } from "../lib/i18n";

const APP_UPDATE_DISMISSED_KEY = "msfs-addons-browser:updater:dismissed-version";

/**
 * Campana de **Notificaciones** del header. Antes mostraba sólo
 * actualizaciones de paquetes Community; ahora también incluye:
 *   · Nueva versión de la app disponible (top del listado).
 *   · Updates de paquetes (como antes).
 *
 * El refresh es automático — el bootstrap del splash ya ejecutó la
 * pasada activa, y al ganar foco la ventana se refresca de nuevo.
 * Por eso aquí no hay botón "Refrescar" — todo es auto.
 */
export function NotificationsBell() {
  const updates = useCommunityStore((s) => s.updates);
  const lastRefreshError = useCommunityStore((s) => s.lastRefreshError);
  const setFocused = useCommunityStore((s) => s.setFocused);
  const dismissUpdate = useCommunityStore((s) => s.dismissUpdate);
  const dismissAll = useCommunityStore((s) => s.dismissAll);
  const startUpdateAll = useCommunityStore((s) => s.startUpdateAll);
  const setView = useAppStore((s) => s.setView);
  const triggerSearch = useAppStore((s) => s.triggerSearch);
  const requestCrewVs = useAppStore((s) => s.requestCrewVs);

  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const [appUpdate, setAppUpdate] = useState<UpdateInfo | null>(null);
  const [appUpdateDismissed, setAppUpdateDismissed] = useState(false);

  // (v6.2.4) Update de GSX (FSDreamTeam) — el chequeo horario vive en App.tsx;
  // aquí sólo leemos el resultado compartido. (v6.2.6) Persiste hasta actualizar
  // (no descartable).
  const gsxInfo = useGsxUpdateStore((s) => s.info);
  const gsxRequestInstall = useGsxUpdateStore((s) => s.requestInstall);
  const showGsx = gsxUpdateVisible(gsxInfo);

  // (v6.2.8) Update de AIRAC (datos de navegación). Persiste hasta actualizar.
  const airacInfo = useAiracUpdateStore((s) => s.info);
  const airacOpenUpdater = useAiracUpdateStore((s) => s.openUpdater);
  const showAirac = airacUpdateVisible(airacInfo);

  // (v6.2.60) Updates de addons descargados de flightsim.to por el embebido.
  // Clic → abre la página del addon (re-descargar) y avanza el baseline; la X
  // sólo lo descarta (también avanza el baseline).
  const flightsimUpdatesMap = useFlightsimUpdateStore((s) => s.updates);
  const ackFlightsimUpdate = useFlightsimUpdateStore((s) => s.ackUpdate);
  const refreshFlightsimUpdates = useFlightsimUpdateStore((s) => s.refreshUpdates);

  // (v6.2.69) Ofertas de perfiles GSX cerradas sin elegir → aviso en la campana;
  // clic reabre el modal, X descarta.
  const gsxPendingOffers = useGsxOfferStore((s) => s.pending);
  const reopenGsxOffer = useGsxOfferStore((s) => s.reopen);
  const clearGsxOffer = useGsxOfferStore((s) => s.clearPending);

  // (v6.2.11) Avisos de "Crew VS listo" (duelos completados, ambos aterrizaron).
  const duelNotices = useLiveVsStore((s) => s.duelNotices);
  const dismissDuelNotice = useLiveVsStore((s) => s.dismissDuelNotice);

  // (v6.2.19) Aviones próximos a mantenimiento (>=75% de desgaste). Clic abre
  // el taller de esa matrícula en Finanzas; la X descarta (reaparece si el
  // desgaste sigue subiendo).
  const maintStart = useMaintAlertsStore((s) => s.start);
  const maintAlertsAll = useMaintAlertsStore((s) => s.alerts);
  const maintDismissed = useMaintAlertsStore((s) => s.dismissed);
  const maintDismiss = useMaintAlertsStore((s) => s.dismiss);
  const openEconomy = useAppStore((s) => s.openEconomy);
  useEffect(() => {
    maintStart();
  }, [maintStart]);
  const maintAlerts = maintAlertsAll.filter((a) => {
    const at = maintDismissed[`${a.registration.toUpperCase()}|${a.component}`];
    return at == null || a.wearPct >= at + 5;
  });

  // (v6.2.20) Escenario recién instalado con "FPS Optimization" disponible.
  // Clic abre el modal de rendimiento (igual que la tuerca del Link Map).
  const fpsStart = useFpsNoticesStore((s) => s.start);
  const fpsNotices = useFpsNoticesStore((s) => s.notices);
  const fpsDismiss = useFpsNoticesStore((s) => s.dismiss);
  const communityPackages = useCommunityStore((s) => s.packages);
  // (v6.2.63) Sólo mostramos updates de flightsim.to de addons AÚN instalados
  // (defensivo: cubre desinstalación manual fuera de la app; la desinstalación
  // por la app ya borra la fila de tracking en el backend).
  const flightsimUpdates = Array.from(flightsimUpdatesMap.values()).filter((u) =>
    communityPackages.some((p) => p.folderName === u.folderName),
  );
  const [perfPkg, setPerfPkg] = useState<CommunityPackage | null>(null);
  useEffect(() => {
    fpsStart();
  }, [fpsStart]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [open]);

  // Comprobamos updates de la app una vez al montar.
  useEffect(() => {
    let cancelled = false;
    api
      .checkForUpdate()
      .then((u) => {
        if (cancelled || !u) return;
        const dismissed = localStorage.getItem(APP_UPDATE_DISMISSED_KEY);
        if (dismissed === u.latestVersion) {
          setAppUpdateDismissed(true);
        }
        setAppUpdate(u);
      })
      .catch((e) => console.warn("checkForUpdate failed:", e));
    return () => {
      cancelled = true;
    };
  }, []);

  const showAppUpdate = appUpdate !== null && !appUpdateDismissed;
  // (v6.2.34) Pre-vuelo pendiente — persiste en la campanita hasta que
  // el usuario resuelve o marca "usar por defecto" cada punto.
  const preflightIssues = usePreflight().result.issues;
  const openPreflight = usePreflightStore((s) => s.setOpen);
  const showPreflight = preflightIssues > 0;

  const packageCount = updates.length;
  const totalCount =
    packageCount +
    (showAppUpdate ? 1 : 0) +
    (showGsx ? 1 : 0) +
    (showAirac ? 1 : 0) +
    (showPreflight ? 1 : 0) +
    duelNotices.length +
    maintAlerts.length +
    fpsNotices.length +
    flightsimUpdates.length +
    gsxPendingOffers.length;

  // Peek-on-first-launch: si hay notificaciones y no hemos peekado
  // antes, abrimos brevemente para que el usuario las vea.
  // (v6.2.37 B4) El aviso de pre-vuelo NO cuenta para el peek: es un
  // recordatorio persistente (ya se ve en el badge y en el Dashboard),
  // no debe forzar la apertura del panel.
  const peekCount = totalCount - (showPreflight ? 1 : 0);
  useEffect(() => {
    if (peekCount === 0) return;
    const PEEK_KEY = "msfs-addons:notifications-peeked";
    if (typeof window === "undefined") return;
    if (window.sessionStorage.getItem(PEEK_KEY)) return;
    window.sessionStorage.setItem(PEEK_KEY, "1");
    setOpen(true);
    const t = window.setTimeout(() => setOpen(false), 4000);
    return () => window.clearTimeout(t);
  }, [peekCount]);

  const focusOnMap = (folderName: string) => {
    setView("map");
    setFocused(folderName);
    setOpen(false);
  };

  const openInSearch = (u: AvailableUpdate) => {
    // El SQL de la rama AIRCRAFT emite `icao = ""` para paquetes
    // no-SCENERY (aviones, liveries, sounds). Sin este helper,
    // clicar una update de A350/A320/etc. lanzaba `search("", source)`
    // y caía en "Sin resultados para """. Ahora caemos a un keyword
    // del título cuando el icao no es plausible.
    const query = deriveUpdateSearchQuery(u.icao, u.title);
    triggerSearch(query, u.source);
    setOpen(false);
  };

  const handleUpdateAll = () => {
    startUpdateAll();
    setOpen(false);
  };

  const dismissAppUpdate = () => {
    if (!appUpdate) return;
    localStorage.setItem(APP_UPDATE_DISMISSED_KEY, appUpdate.latestVersion);
    setAppUpdateDismissed(true);
  };

  return (
    <div ref={wrapRef} className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        className="relative inline-flex h-9 w-9 items-center justify-center rounded-lg border border-slate-800 bg-slate-900/60 text-slate-300 transition-colors hover:border-brand-500/40 hover:text-slate-100"
        title={
          totalCount > 0
            ? `${totalCount} notificación(es)`
            : "Sin notificaciones"
        }
      >
        <Bell className="h-4 w-4" />
        {totalCount > 0 && (
          <span className="absolute -right-1 -top-1 inline-flex h-4 min-w-[1rem] items-center justify-center rounded-full bg-amber-500 px-1 text-[10px] font-bold text-amber-950 ring-2 ring-slate-950">
            {totalCount > 99 ? "99+" : totalCount}
          </span>
        )}
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, y: -4, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -4, scale: 0.98 }}
            transition={{ duration: 0.12 }}
            className="absolute right-0 top-full z-30 mt-2 w-[420px] origin-top-right overflow-hidden rounded-xl border border-slate-800 bg-slate-950/95 shadow-xl ring-1 ring-slate-800 backdrop-blur"
          >
            <header className="flex items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
              <div>
                <h3 className="text-sm font-semibold text-slate-100">
                  {t("notifications.title")}
                </h3>
              </div>
              <div className="flex items-center gap-1.5">
                {/* (DEBUG v6.2.60, temporal) Fuerza updates de prueba de
                    flightsim.to para verificar el flujo campana/badge. */}
                <button
                  onClick={() => {
                    void api
                      .flightsimDebugForceUpdate()
                      .then((n) => {
                        console.info(`[debug] flightsim force-update: ${n} filas`);
                        return refreshFlightsimUpdates();
                      })
                      .catch((e) => console.warn("debug force-update failed:", e));
                  }}
                  title="DEBUG: forzar update de prueba de flightsim.to"
                  className="rounded-md border border-sky-800 p-1.5 text-sky-300 hover:border-sky-500/40 hover:bg-sky-900/30"
                >
                  <FlaskConical className="h-3.5 w-3.5" />
                </button>
                <button
                  onClick={() => {
                    void api
                      .checkForUpdate()
                      .then((u) => {
                        setAppUpdateDismissed(false);
                        setAppUpdate(u);
                      })
                      .catch((e) => console.warn("re-check failed:", e));
                  }}
                  title={t("notifications.recheck_tooltip")}
                  className="rounded-md border border-slate-800 p-1.5 text-slate-300 hover:border-brand-500/40 hover:bg-slate-900"
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                </button>
                {packageCount > 0 && (
                  <button
                    onClick={() => dismissAll()}
                    title={t("notifications.mark_all_seen")}
                    className="rounded-md border border-slate-800 p-1.5 text-slate-300 hover:border-brand-500/40 hover:bg-slate-900"
                  >
                    <CheckCheck className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>
            </header>

            {/* (v6.2.33 fix) Contenedor scrolleable: con muchas
                notificaciones antes no se podía ver las de abajo sin ir
                borrando las de arriba. */}
            <div className="max-h-[min(65vh,560px)] overflow-y-auto">
            {/* (v6.2.34) Pre-vuelo pendiente — persiste hasta resolver. */}
            {showPreflight && (
              <button
                onClick={() => {
                  openPreflight(true);
                  setOpen(false);
                }}
                className="flex w-full items-center gap-3 border-b border-emerald-500/20 bg-emerald-500/10 px-4 py-3 text-left hover:bg-emerald-500/15"
              >
                <Rocket className="h-4 w-4 shrink-0 text-emerald-300" />
                <div className="min-w-0 flex-1">
                  <div className="text-xs font-semibold text-emerald-100">
                    {t("preflight.cta.title")}
                  </div>
                  <div className="mt-0.5 text-[11px] text-emerald-200/80">
                    {t("preflight.cta.issues", { n: String(preflightIssues) })}
                  </div>
                </div>
              </button>
            )}
            {/* App update — pinned al top, en verde para diferenciar
                de los updates de paquetes (ámbar). */}
            {showAppUpdate && appUpdate && (
              <div className="border-b border-emerald-500/20 bg-emerald-500/10 px-4 py-3">
                <div className="flex items-start gap-3">
                  <Sparkles className="mt-0.5 h-4 w-4 shrink-0 text-emerald-300" />
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-semibold text-emerald-100">
                      {t("notifications.new_app_version")}
                    </div>
                    <div className="mt-0.5 text-[11px] text-emerald-200/80">
                      {appUpdate.currentVersion} →{" "}
                      <strong>{appUpdate.latestVersion}</strong>
                    </div>
                    <div className="mt-2 flex items-center gap-1.5">
                      {appUpdate.assetUrl ? (
                        <button
                          onClick={() => api.openExternal(appUpdate.assetUrl!)}
                          className="inline-flex items-center gap-1 rounded-md bg-emerald-500/30 px-2.5 py-1 text-[11px] font-medium text-emerald-100 hover:bg-emerald-500/40"
                        >
                          <Download className="h-3 w-3" /> {t("notifications.download")}
                        </button>
                      ) : (
                        <button
                          onClick={() => api.openExternal(appUpdate.releaseUrl)}
                          className="inline-flex items-center gap-1 rounded-md bg-emerald-500/30 px-2.5 py-1 text-[11px] font-medium text-emerald-100 hover:bg-emerald-500/40"
                        >
                          <ExternalLink className="h-3 w-3" /> {t("notifications.view_release")}
                        </button>
                      )}
                    </div>
                  </div>
                  <button
                    onClick={dismissAppUpdate}
                    title={t("notifications.dismiss_app_update")}
                    className="shrink-0 rounded-md p-1 text-emerald-200/70 hover:bg-emerald-500/20 hover:text-emerald-100"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            )}

            {/* (v6.2.4) Update de GSX — en cyan, distinto de app (verde) y
                paquetes (ámbar). Al actualizar abre el FSDT Installer. */}
            {showGsx && gsxInfo && (
              <div className="border-b border-cyan-500/20 bg-cyan-500/10 px-4 py-3">
                <div className="flex items-start gap-3">
                  <Truck className="mt-0.5 h-4 w-4 shrink-0 text-cyan-300" />
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-semibold text-cyan-100">
                      {t("notifications.gsx_update")}
                    </div>
                    <div className="mt-0.5 text-[11px] text-cyan-200/80">
                      {gsxInfo.installedVersion} →{" "}
                      <strong>{gsxInfo.latestVersion}</strong>
                      {gsxInfo.latestDate ? ` · ${gsxInfo.latestDate}` : ""}
                    </div>
                    <div className="mt-2 flex items-center gap-1.5">
                      <button
                        onClick={() => {
                          gsxRequestInstall();
                          setOpen(false);
                        }}
                        className="inline-flex items-center gap-1 rounded-md bg-cyan-500/30 px-2.5 py-1 text-[11px] font-medium text-cyan-100 hover:bg-cyan-500/40"
                      >
                        <Download className="h-3 w-3" />{" "}
                        {t("notifications.gsx_update_now")}
                      </button>
                      <button
                        onClick={() => api.openExternal(gsxInfo.notesUrl)}
                        className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-cyan-200/80 hover:bg-cyan-500/20 hover:text-cyan-100"
                      >
                        <ExternalLink className="h-3 w-3" />{" "}
                        {t("notifications.gsx_notes")}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* (v6.2.8) Update de AIRAC — en índigo. Clic abre Navigraph Hub. */}
            {showAirac && airacInfo && (
              <div className="border-b border-indigo-500/20 bg-indigo-500/10 px-4 py-3">
                <div className="flex items-start gap-3">
                  <Navigation className="mt-0.5 h-4 w-4 shrink-0 text-indigo-300" />
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-semibold text-indigo-100">
                      {t("notifications.airac_update")}
                    </div>
                    <div className="mt-0.5 text-[11px] text-indigo-200/80">
                      {t("notifications.airac_cycle")} {airacInfo.installedCycle}{" "}
                      → <strong>{airacInfo.latestCycle}</strong>
                      {airacInfo.effectiveDate
                        ? ` · ${airacInfo.effectiveDate}`
                        : ""}
                    </div>
                    <div className="mt-2 flex items-center gap-1.5">
                      <button
                        onClick={() => {
                          void airacOpenUpdater();
                          setOpen(false);
                        }}
                        className="inline-flex items-center gap-1 rounded-md bg-indigo-500/30 px-2.5 py-1 text-[11px] font-medium text-indigo-100 hover:bg-indigo-500/40"
                      >
                        <Download className="h-3 w-3" />{" "}
                        {t("notifications.airac_update_now")}
                      </button>
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* (v6.2.60) Updates de addons descargados de flightsim.to — en
                sky/azul. Clic → abre la página del addon en el embebido para
                re-descargar (y avanza el baseline); la X sólo lo descarta. */}
            {flightsimUpdates.map((u) => (
              <div
                key={u.folderName}
                className="border-b border-sky-500/20 bg-sky-500/10 px-4 py-3"
              >
                <div className="flex items-start gap-3">
                  <Download className="mt-0.5 h-4 w-4 shrink-0 text-sky-300" />
                  <button
                    onClick={() => {
                      ackFlightsimUpdate(u.folderName);
                      void api.openLiveryBrowser(u.link);
                      setOpen(false);
                    }}
                    className="min-w-0 flex-1 text-left"
                  >
                    <div className="text-xs font-semibold text-sky-100">
                      {t("notifications.fsto_update")}
                    </div>
                    <div className="mt-0.5 truncate text-[11px] text-sky-200/80">
                      {u.title ?? u.folderName}
                    </div>
                    {(u.currentVersion || u.latestVersion) && (
                      <div className="mt-1 inline-flex items-center gap-1 rounded bg-sky-500/15 px-2 py-0.5 text-[11px] font-medium text-sky-200 ring-1 ring-sky-500/30">
                        v{u.currentVersion ?? "?"} → v{u.latestVersion ?? "?"}
                      </div>
                    )}
                  </button>
                  <button
                    onClick={() => ackFlightsimUpdate(u.folderName)}
                    title={t("common.dismiss")}
                    className="shrink-0 rounded-md p-1 text-sky-200/70 hover:bg-sky-500/20 hover:text-sky-100"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            ))}

            {/* (v6.2.69) Ofertas de perfiles GSX cerradas sin elegir — en cyan.
                Clic reabre el modal con las opciones; la X descarta. */}
            {gsxPendingOffers.map((o) => (
              <div
                key={o.icao}
                className="border-b border-cyan-500/20 bg-cyan-500/10 px-4 py-3"
              >
                <div className="flex items-start gap-3">
                  <Truck className="mt-0.5 h-4 w-4 shrink-0 text-cyan-300" />
                  <button
                    onClick={() => {
                      reopenGsxOffer(o.icao);
                      setOpen(false);
                    }}
                    className="min-w-0 flex-1 text-left"
                  >
                    <div className="text-xs font-semibold text-cyan-100">
                      {t("gsxoffer.title", { icao: o.icao })}
                    </div>
                    <div className="mt-0.5 truncate text-[11px] text-cyan-200/80">
                      {o.airportTitle}
                      {o.profiles.length > 0
                        ? ` · ${t("gsxoffer.subtitle", { n: String(o.profiles.length) })}`
                        : ""}
                    </div>
                  </button>
                  <button
                    onClick={() => clearGsxOffer(o.icao)}
                    title={t("common.dismiss")}
                    className="shrink-0 rounded-md p-1 text-cyan-200/70 hover:bg-cyan-500/20 hover:text-cyan-100"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            ))}

            {/* (v6.2.11) Crew VS listo — duelos completados (ambos aterrizaron).
                Clic abre el FlightBook para revisarlo; la X lo descarta. */}
            {duelNotices.map((d) => (
              <div
                key={d.channel}
                className="border-b border-fuchsia-500/20 bg-fuchsia-500/10 px-4 py-3"
              >
                <div className="flex items-start gap-3">
                  <Swords className="mt-0.5 h-4 w-4 shrink-0 text-fuchsia-300" />
                  <button
                    onClick={() => {
                      // (v6.2.35) Abre DIRECTO el Crew VS de ese vuelo (no
                      // sólo el FlightBook) y descarta el aviso.
                      requestCrewVs(d.origin, d.dest, d.at);
                      dismissDuelNotice(d.channel);
                      setOpen(false);
                    }}
                    className="min-w-0 flex-1 text-left"
                  >
                    <div className="text-xs font-semibold text-fuchsia-100">
                      {t("notifications.vs_ready")}
                    </div>
                    <div className="mt-0.5 font-mono text-[11px] text-fuchsia-200/80">
                      {d.origin} → {d.dest}
                    </div>
                    <div className="mt-0.5 text-[11px] text-fuchsia-200/80">
                      {t("vs.you")} {Math.round(d.selfFpm)} · {d.rivalName}{" "}
                      {Math.round(d.rivalFpm)} fpm
                    </div>
                  </button>
                  <button
                    onClick={() => dismissDuelNotice(d.channel)}
                    title={t("notifications.dismiss_app_update")}
                    className="shrink-0 rounded-md p-1 text-fuchsia-200/70 hover:bg-fuchsia-500/20 hover:text-fuchsia-100"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            ))}

            {/* (v6.2.19) Aviones próximos a mantenimiento. Clic → taller de la
                matrícula en Finanzas. Ámbar = pronto (75-80), rojo = vencido. */}
            {maintAlerts.map((a) => {
              const due = a.severity === "due";
              return (
                <div
                  key={`${a.registration}|${a.component}`}
                  className={`border-b px-4 py-3 ${
                    due
                      ? "border-rose-500/20 bg-rose-500/10"
                      : "border-amber-500/20 bg-amber-500/10"
                  }`}
                >
                  <div className="flex items-start gap-3">
                    <Wrench
                      className={`mt-0.5 h-4 w-4 shrink-0 ${
                        due ? "text-rose-300" : "text-amber-300"
                      }`}
                    />
                    <button
                      onClick={() => {
                        openEconomy(a.airlineKey, a.registration);
                        setOpen(false);
                      }}
                      className="min-w-0 flex-1 text-left"
                    >
                      <div
                        className={`text-xs font-semibold ${
                          due ? "text-rose-100" : "text-amber-100"
                        }`}
                      >
                        {due
                          ? t("notifications.maint_due")
                          : t("notifications.maint_soon")}
                      </div>
                      <div className="mt-0.5 font-mono text-[11px] text-slate-300">
                        {a.registration}
                        {a.model ? ` · ${a.model}` : ""}
                      </div>
                      <div className="mt-0.5 text-[11px] text-slate-400">
                        {t(`economy.comp.${a.component}`)} ·{" "}
                        {Math.round(a.wearPct)}%
                        {a.dueCount > 1
                          ? ` · +${a.dueCount - 1} ${t("notifications.maint_more")}`
                          : ""}
                      </div>
                    </button>
                    <button
                      onClick={() => maintDismiss(a)}
                      title={t("notifications.dismiss_app_update")}
                      className="shrink-0 rounded-md p-1 text-slate-400 hover:bg-slate-700/40 hover:text-slate-200"
                    >
                      <X className="h-3.5 w-3.5" />
                    </button>
                  </div>
                </div>
              );
            })}

            {/* (v6.2.20) FPS Optimization disponible en escenario recién
                instalado — clic abre el modal de rendimiento. */}
            {fpsNotices.map((n) => (
              <div
                key={n.folderName}
                className="border-b border-emerald-500/20 bg-emerald-500/10 px-4 py-3"
              >
                <div className="flex items-start gap-3">
                  <Gauge className="mt-0.5 h-4 w-4 shrink-0 text-emerald-300" />
                  <button
                    onClick={() => {
                      const pkg = communityPackages.find(
                        (p) => p.folderName === n.folderName,
                      );
                      if (pkg) {
                        setPerfPkg(pkg);
                        setOpen(false);
                      } else {
                        // Ya no está instalado — el aviso sobra.
                        fpsDismiss(n.folderName);
                      }
                    }}
                    className="min-w-0 flex-1 text-left"
                  >
                    <div className="text-xs font-semibold text-emerald-100">
                      {n.reason === "updated"
                        ? t("notifications.fps_after_update")
                        : t("notifications.fps_available")}
                    </div>
                    <div className="mt-0.5 truncate text-[11px] text-emerald-200/80">
                      {n.title}
                    </div>
                    <div className="mt-0.5 text-[11px] text-slate-400">
                      {n.reason === "updated"
                        ? t("notifications.fps_after_update_hint")
                        : t("notifications.fps_hint")}
                    </div>
                  </button>
                  <button
                    onClick={() => fpsDismiss(n.folderName)}
                    title={t("notifications.dismiss_app_update")}
                    className="shrink-0 rounded-md p-1 text-emerald-200/70 hover:bg-emerald-500/20 hover:text-emerald-100"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            ))}

            {packageCount > 0 && (
              <div className="border-b border-slate-800 bg-amber-500/5 px-4 py-2">
                <button
                  onClick={handleUpdateAll}
                  className="inline-flex w-full items-center justify-center gap-2 rounded-md bg-amber-500 px-3 py-2 text-xs font-bold uppercase tracking-wide text-amber-950 shadow-md shadow-amber-500/20 hover:bg-amber-400"
                >
                  <Sparkles className="h-3.5 w-3.5" />
                  {t("notifications.update_all")} ({packageCount})
                </button>
                <p className="mt-1 text-center text-[10px] text-slate-500">
                  {t("notifications.update_all.hint")}
                </p>
              </div>
            )}

            <div>
              {lastRefreshError && (
                <div className="m-3 rounded bg-rose-500/15 px-3 py-2 text-xs text-rose-300 ring-1 ring-rose-500/30">
                  {lastRefreshError}
                </div>
              )}
              {totalCount === 0 ? (
                <div className="px-4 py-8 text-center text-xs text-slate-500">
                  {t("notifications.empty")}
                </div>
              ) : packageCount === 0 ? null : (
                <ul className="divide-y divide-slate-800">
                  {updates.map((u) => (
                    <li
                      key={u.folderName}
                      className="group flex items-stretch hover:bg-slate-900/60"
                    >
                      <button
                        onClick={() => openInSearch(u)}
                        className="flex flex-1 items-start gap-3 px-4 py-3 text-left"
                        title={`Buscar ${u.icao || u.title} en ${u.source} para descargar la nueva versión`}
                      >
                        <div className="mt-1 shrink-0">
                          <Download className="h-4 w-4 text-amber-300" />
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            <span className="font-mono text-[11px] font-semibold text-brand-300">
                              {u.icao}
                            </span>
                            <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-slate-400">
                              {u.source}
                            </span>
                          </div>
                          <div className="mt-1 truncate text-sm text-slate-100">
                            {u.title}
                          </div>
                          <div className="mt-1 inline-flex items-center gap-1 rounded bg-amber-500/15 px-2 py-0.5 text-[11px] font-medium text-amber-300 ring-1 ring-amber-500/30">
                            v{u.installedVersion} → v{u.latestVersion}
                          </div>
                        </div>
                      </button>
                      <div className="flex flex-col items-center gap-1 px-2 py-3">
                        <button
                          onClick={() => focusOnMap(u.folderName)}
                          title={t("common.view_on_map")}
                          className="rounded-md border border-slate-800 p-1 text-slate-400 hover:border-brand-500/40 hover:text-slate-100"
                        >
                          <MapIcon className="h-3.5 w-3.5" />
                        </button>
                        <button
                          onClick={() => dismissUpdate(u.folderName)}
                          title={t("common.dismiss")}
                          className="rounded-md border border-slate-800 p-1 text-slate-400 hover:border-rose-500/40 hover:bg-rose-500/10 hover:text-rose-200"
                        >
                          <X className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* (v6.2.21) Modal de rendimiento abierto desde un aviso de FPS — mismo
          componente que la tuerca del Link Map, pero por PORTAL a <body>: el
          header tiene backdrop-blur/transform y atrapaba el `fixed` del modal
          (salía arriba, recortado e incerrable — reportado). */}
      {perfPkg &&
        createPortal(
          <PerformanceModal pkg={perfPkg} onClose={() => setPerfPkg(null)} />,
          document.body,
        )}
    </div>
  );
}
