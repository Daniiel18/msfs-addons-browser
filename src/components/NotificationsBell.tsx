import { useEffect, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import {
  Bell,
  BellRing,
  CheckCheck,
  Download,
  ExternalLink,
  Map as MapIcon,
  RefreshCw,
  Sparkles,
  Truck,
  Navigation,
  Rocket,
  Swords,
  Wrench,
  Gauge,
  Package,
  X,
} from "lucide-react";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useAppStore } from "../stores/useAppStore";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import {
  useGsxUpdateStore,
  gsxUpdateVisible,
} from "../stores/useGsxUpdateStore";
import {
  useAiracUpdateStore,
  airacUpdateVisible,
} from "../stores/useAiracUpdateStore";
import { useLiveVsStore } from "../stores/useLiveVsStore";
import { useFlightsimUpdateStore, useInstalledFlightsimUpdates } from "../stores/useFlightsimUpdateStore";
import { useFstoEmbedStore } from "./FlightsimEmbed";
import { updateOnMap, usePendingUpdatesModal } from "./PendingUpdatesModal";
import { openInFlightsimTab } from "../lib/flightsimTab";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { useGsxOfferStore } from "../stores/useGsxOfferStore";
import { useMaintAlertsStore } from "../stores/useMaintAlertsStore";
import { useFpsNoticesStore } from "../stores/useFpsNoticesStore";
import {
  useInstallQueueStore,
  batchLabel,
  batchItemCount,
} from "../stores/useInstallQueueStore";
import { installPendingNow, cancelPending } from "../lib/installQueue";
import { usePreflight } from "../lib/usePreflight";
import { usePreflightStore } from "../stores/usePreflightStore";
import { PerformanceModal } from "./PerformanceModal";
import type { CommunityPackage, UpdateInfo } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";

const APP_UPDATE_DISMISSED_KEY = "msfs-addons-browser:updater:dismissed-version";
/** Cuántas actualizaciones de addons se ven en la campanita (el resto, en el modal). */
const ADDON_PREVIEW = 3;

/**
 * Campana de **Notificaciones** del header.
 *
 * (26/10) Etapa 7 — rediseño: panel lateral como Descargas, tarjetas neutras
 * con su icono de color (o la imagen del addon) y grupos con cabecera:
 * Pendiente · Actualizaciones · Flota y vuelos · Sugerencias. De los addons
 * con versión nueva salen las 3 primeras y el resto en el modal de tarjetas.
 * Al arrancar ya no se abre el panel: un globo junto a la campanita avisa.
 *
 * El refresh es automático — el bootstrap del splash ya ejecutó la pasada
 * activa, y al ganar foco la ventana se refresca de nuevo.
 */
export function NotificationsBell() {
  const updates = useCommunityStore((s) => s.updates);
  const lastRefreshError = useCommunityStore((s) => s.lastRefreshError);
  const setFocused = useCommunityStore((s) => s.setFocused);
  const dismissUpdate = useCommunityStore((s) => s.dismissUpdate);
  const dismissAll = useCommunityStore((s) => s.dismissAll);
  const startUpdateAll = useCommunityStore((s) => s.startUpdateAll);
  const setView = useAppStore((s) => s.setView);
  const openCatalogAddon = useAppStore((s) => s.openCatalogAddon);
  const openFlightsimTab = useAppStore((s) => s.openFlightsimTab);
  const requestCrewVs = useAppStore((s) => s.requestCrewVs);
  const openUpdatesModal = usePendingUpdatesModal((s) => s.setOpen);

  const [open, setOpen] = useState(false);
  const [peek, setPeek] = useState(false);

  // (Rediseño F2) Otras pantallas (Inicio › Avisos) pueden abrir la campana.
  useEffect(() => {
    const onOpen = () => setOpen(true);
    window.addEventListener("simfleet:notifications", onOpen);
    return () => window.removeEventListener("simfleet:notifications", onOpen);
  }, []);
  // (26/10) Un solo panel lateral a la vez: Descargas cierra éste y viceversa.
  const downloadsOpen = useDownloadsStore((s) => s.isPanelOpen);
  useEffect(() => {
    if (downloadsOpen) setOpen(false);
  }, [downloadsOpen]);
  useEffect(() => {
    if (!open) return;
    setPeek(false);
    useDownloadsStore.getState().setPanelOpen(false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  const [appUpdate, setAppUpdate] = useState<UpdateInfo | null>(null);
  const [appUpdateDismissed, setAppUpdateDismissed] = useState(false);

  // (v6.2.4) Update de GSX (FSDreamTeam) — el chequeo horario vive en App.tsx;
  // aquí sólo leemos el resultado compartido. Persiste hasta actualizar.
  const gsxInfo = useGsxUpdateStore((s) => s.info);
  const gsxRequestInstall = useGsxUpdateStore((s) => s.requestInstall);
  const showGsx = gsxUpdateVisible(gsxInfo);

  // (v6.2.8) Update de AIRAC (datos de navegación). Persiste hasta actualizar.
  const airacInfo = useAiracUpdateStore((s) => s.info);
  const airacOpenUpdater = useAiracUpdateStore((s) => s.openUpdater);
  const showAirac = airacUpdateVisible(airacInfo);

  // (v6.2.60) Updates de addons descargados de flightsim.to.
  const ackFlightsimUpdate = useFlightsimUpdateStore((s) => s.ackUpdate);

  // (v6.2.69) Ofertas de perfiles GSX cerradas sin elegir.
  const gsxPendingOffers = useGsxOfferStore((s) => s.pending);
  const reopenGsxOffer = useGsxOfferStore((s) => s.reopen);
  const clearGsxOffer = useGsxOfferStore((s) => s.clearPending);

  // (v6.2.11) Avisos de "Crew VS listo" (duelos completados, ambos aterrizaron).
  const duelNotices = useLiveVsStore((s) => s.duelNotices);
  const dismissDuelNotice = useLiveVsStore((s) => s.dismissDuelNotice);

  // (v6.2.19) Aviones con mantenimiento pendiente. La X descarta (reaparece
  // si el desgaste sigue subiendo).
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
  const fpsStart = useFpsNoticesStore((s) => s.start);
  const fpsNotices = useFpsNoticesStore((s) => s.notices);
  const fpsDismiss = useFpsNoticesStore((s) => s.dismiss);
  const communityPackages = useCommunityStore((s) => s.packages);
  const flightsimUpdates = useInstalledFlightsimUpdates();
  // (26/10) Perfiles GSX instalados con versión nueva (cuenta de flightsim.to).
  const gsxProfileUpdatesMap = useGsxLocalStore((s) => s.updates);
  const gsxProfileUpdates = Array.from(gsxProfileUpdatesMap.values());
  const ackGsxProfile = useGsxLocalStore((s) => s.ackUpdate);
  const [perfPkg, setPerfPkg] = useState<CommunityPackage | null>(null);
  useEffect(() => {
    fpsStart();
  }, [fpsStart]);

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
  // (v6.2.34) Pre-vuelo pendiente — persiste hasta resolver cada punto.
  const preflightIssues = usePreflight().result.issues;
  const openPreflight = usePreflightStore((s) => s.setOpen);
  const showPreflight = preflightIssues > 0;

  // (Ajustes 19/10) Instalaciones aparcadas (clic fuera del modal).
  const pendingInstalls = useInstallQueueStore((s) => s.pending);
  const openPendingInstall = useInstallQueueStore((s) => s.openPending);

  const packageCount = updates.length;
  const close = () => setOpen(false);

  // Actualizaciones de addons (catálogos, flightsim.to y perfiles GSX), en una
  // sola lista: aquí las primeras, todas en el modal de tarjetas.
  const addonItems: AddonItem[] = [
    ...updates.map((u) => ({
      key: `cat:${u.folderName}`,
      title: u.title,
      meta: [u.icao, u.source].filter(Boolean).join(" · "),
      image: u.imageUrl,
      from: u.installedVersion,
      to: u.latestVersion,
      tooltip: t("updates.modal.go_tooltip", { source: u.source }),
      onOpen: () => {
        void openCatalogAddon(u);
        close();
      },
      onMap: updateOnMap(u, communityPackages)
        ? () => {
            setView("map");
            setFocused(u.folderName);
            close();
          }
        : undefined,
      onDismiss: () => void dismissUpdate(u.folderName),
    })),
    ...flightsimUpdates.map((u) => ({
      key: `fsto:${u.folderName}`,
      title: u.title ?? u.folderName,
      meta: "flightsim.to",
      image: u.thumbnail ?? null,
      from: u.currentVersion ?? "?",
      to: u.latestVersion ?? "?",
      tooltip: t("updates.modal.go_fsto_tooltip"),
      onOpen: () => {
        // (25/10) En la pestaña flightsim.to de Explorar, como los demás catálogos.
        ackFlightsimUpdate(u.folderName);
        useFstoEmbedStore.getState().openUrl(u.link);
        openFlightsimTab();
        close();
      },
      onDismiss: () => ackFlightsimUpdate(u.folderName),
    })),
    ...gsxProfileUpdates.map((g) => ({
      key: `gsx:${g.icao}`,
      title: g.title || t("updates.modal.gsx_title", { icao: g.icao }),
      meta: `${g.icao} · ${t("updates.modal.gsx_source")}`,
      image: g.thumbnail ?? null,
      from: "",
      to: g.latestVersion ?? "",
      tooltip: t("updates.modal.go_fsto_tooltip"),
      onOpen: () => {
        ackGsxProfile(g.icao);
        openInFlightsimTab(g.link);
        close();
      },
      onDismiss: () => ackGsxProfile(g.icao),
    })),
  ];

  const pendingCount = pendingInstalls.length + (showPreflight ? 1 : 0);
  const updatesCount = (showAppUpdate ? 1 : 0) + (showGsx ? 1 : 0) + (showAirac ? 1 : 0) + addonItems.length;
  const fleetCount = maintAlerts.length + duelNotices.length;
  const suggestCount = fpsNotices.length + gsxPendingOffers.length;
  const totalCount = pendingCount + updatesCount + fleetCount + suggestCount;

  // Al arrancar, si hay avisos, un globo junto a la campanita (antes se abría
  // el panel 4 s y tapaba lo que hubiera debajo). El pre-vuelo no cuenta: es
  // un recordatorio persistente (ya se ve en el badge y en Inicio).
  // (25/10) El cierre no se cancela: cerrar tras desmontar no hace nada.
  const peekCount = totalCount - (showPreflight ? 1 : 0);
  useEffect(() => {
    if (peekCount === 0) return;
    const PEEK_KEY = "msfs-addons:notifications-peeked";
    if (typeof window === "undefined") return;
    if (window.sessionStorage.getItem(PEEK_KEY)) return;
    window.sessionStorage.setItem(PEEK_KEY, "1");
    setPeek(true);
    window.setTimeout(() => setPeek(false), 6000);
  }, [peekCount]);

  const dismissAppUpdate = () => {
    if (!appUpdate) return;
    localStorage.setItem(APP_UPDATE_DISMISSED_KEY, appUpdate.latestVersion);
    setAppUpdateDismissed(true);
  };

  const panel = (
    <>
      {/* Igual que Descargas: el velo y el panel como hermanos directos de
          AnimatePresence (dentro de un Fragment no se desmontan bien). */}
      <AnimatePresence>
        {open && (
          <motion.div
            key="scrim"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            onClick={close}
            className="fixed inset-x-0 bottom-0 top-11 z-[46] bg-slate-950/70"
          />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {open && (
          <motion.aside
            key="panel"
            initial={{ x: "100%" }}
            animate={{ x: 0 }}
            exit={{ x: "100%" }}
            transition={{ type: "spring", stiffness: 320, damping: 34 }}
            data-sf-overlay
            className="fixed bottom-0 right-0 top-11 z-[47] flex w-full max-w-md flex-col border-l border-sf-border bg-sf-bg shadow-2xl"
          >
            <header className="flex items-center justify-between gap-2 border-b border-sf-border px-5 py-4">
              <div className="flex items-center gap-2">
                <Bell className="h-4 w-4 text-brand-300" />
                <h2 className="text-sm font-semibold text-sf-text">{t("notifications.title")}</h2>
                {totalCount > 0 && (
                  <span className="rounded-full bg-sf-raised px-1.5 py-0.5 text-[11px] tabular-nums text-sf-muted">
                    {totalCount}
                  </span>
                )}
              </div>
              <div className="flex items-center gap-1">
                <IconBtn
                  title={t("notifications.recheck_tooltip")}
                  onClick={() => {
                    void api
                      .checkForUpdate()
                      .then((u) => {
                        setAppUpdateDismissed(false);
                        setAppUpdate(u);
                      })
                      .catch((e) => console.warn("re-check failed:", e));
                  }}
                >
                  <RefreshCw className="h-4 w-4" />
                </IconBtn>
                {packageCount > 0 && (
                  <IconBtn title={t("notifications.mark_all_seen")} onClick={() => dismissAll()}>
                    <CheckCheck className="h-4 w-4" />
                  </IconBtn>
                )}
                <IconBtn title={t("common.close")} onClick={close}>
                  <X className="h-4 w-4" />
                </IconBtn>
              </div>
            </header>

            <div className="flex-1 space-y-5 overflow-y-auto px-4 py-4">
              {lastRefreshError && (
                <div className="rounded-lg bg-rose-500/10 px-3 py-2 text-xs text-rose-300 ring-1 ring-rose-500/30">
                  {lastRefreshError}
                </div>
              )}
              {totalCount === 0 && (
                <div className="mt-16 flex flex-col items-center text-center">
                  <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-sf-surface ring-1 ring-sf-border">
                    <Bell className="h-5 w-5 text-sf-muted" />
                  </div>
                  <p className="mt-3 text-sm text-sf-text/90">{t("notifications.all_clear")}</p>
                  <p className="mt-1 max-w-[280px] text-xs text-sf-muted">{t("notifications.empty")}</p>
                </div>
              )}

              {/* Pendiente: instalaciones aparcadas y pre-vuelo. */}
              {pendingCount > 0 && (
                <Group title={t("notifications.group.pending")} count={pendingCount}>
                  {pendingInstalls.map((b) => (
                    <NoticeCard
                      key={b.id}
                      tone="amber"
                      icon={<Package className="h-4 w-4" />}
                      urgent
                      title={t("installq.pending_title")}
                      lines={[
                        <span key="l" className="font-mono">{batchLabel(b)}</span>,
                        t("installq.items", { n: String(batchItemCount(b)) }),
                      ]}
                      onOpen={() => {
                        openPendingInstall(b.id);
                        close();
                      }}
                      openTitle={t("installq.open_tooltip")}
                      actions={
                        <>
                          <SecondaryBtn onClick={() => void cancelPending(b.id)}>{t("common.cancel")}</SecondaryBtn>
                          <PrimaryBtn onClick={() => void installPendingNow(b.id)}>{t("installq.install")}</PrimaryBtn>
                        </>
                      }
                    />
                  ))}
                  {showPreflight && (
                    <NoticeCard
                      tone="emerald"
                      icon={<Rocket className="h-4 w-4" />}
                      title={t("preflight.cta.title")}
                      lines={[t("preflight.cta.issues", { n: String(preflightIssues) })]}
                      onOpen={() => {
                        openPreflight(true);
                        close();
                      }}
                    />
                  )}
                </Group>
              )}

              {/* Actualizaciones. */}
              {updatesCount > 0 && (
                <Group title={t("notifications.group.updates")} count={updatesCount}>
                  {showAppUpdate && appUpdate && (
                    <NoticeCard
                      tone="emerald"
                      icon={<Sparkles className="h-4 w-4" />}
                      title={t("notifications.new_app_version")}
                      chip={<VersionChip from={appUpdate.currentVersion} to={appUpdate.latestVersion} />}
                      onDismiss={dismissAppUpdate}
                      actions={
                        appUpdate.assetUrl ? (
                          <PrimaryBtn onClick={() => api.openExternal(appUpdate.assetUrl!)}>
                            <Download className="h-3 w-3" /> {t("notifications.download")}
                          </PrimaryBtn>
                        ) : (
                          <SecondaryBtn onClick={() => api.openExternal(appUpdate.releaseUrl)}>
                            <ExternalLink className="h-3 w-3" /> {t("notifications.view_release")}
                          </SecondaryBtn>
                        )
                      }
                    />
                  )}
                  {showGsx && gsxInfo && (
                    <NoticeCard
                      tone="cyan"
                      icon={<Truck className="h-4 w-4" />}
                      title={t("notifications.gsx_update")}
                      chip={<VersionChip from={gsxInfo.installedVersion} to={gsxInfo.latestVersion} />}
                      lines={gsxInfo.latestDate ? [gsxInfo.latestDate] : undefined}
                      actions={
                        <>
                          <SecondaryBtn onClick={() => api.openExternal(gsxInfo.notesUrl)}>
                            <ExternalLink className="h-3 w-3" /> {t("notifications.gsx_notes")}
                          </SecondaryBtn>
                          <PrimaryBtn
                            onClick={() => {
                              gsxRequestInstall();
                              close();
                            }}
                          >
                            <Download className="h-3 w-3" /> {t("notifications.gsx_update_now")}
                          </PrimaryBtn>
                        </>
                      }
                    />
                  )}
                  {showAirac && airacInfo && (
                    <NoticeCard
                      tone="indigo"
                      icon={<Navigation className="h-4 w-4" />}
                      title={t("notifications.airac_update")}
                      chip={
                        <VersionChip
                          plain
                          from={`${t("notifications.airac_cycle")} ${airacInfo.installedCycle}`}
                          to={String(airacInfo.latestCycle)}
                        />
                      }
                      lines={airacInfo.effectiveDate ? [airacInfo.effectiveDate] : undefined}
                      actions={
                        <PrimaryBtn
                          onClick={() => {
                            void airacOpenUpdater();
                            close();
                          }}
                        >
                          <Download className="h-3 w-3" /> {t("notifications.airac_update_now")}
                        </PrimaryBtn>
                      }
                    />
                  )}
                  {addonItems.slice(0, ADDON_PREVIEW).map((a) => (
                    <NoticeCard
                      key={a.key}
                      tone="amber"
                      image={a.image}
                      icon={<Download className="h-4 w-4" />}
                      title={a.title}
                      lines={[a.meta]}
                      chip={<VersionChip from={a.from} to={a.to} />}
                      onOpen={a.onOpen}
                      openTitle={a.tooltip}
                      onDismiss={a.onDismiss}
                      extra={
                        a.onMap && (
                          <button
                            onClick={a.onMap}
                            title={t("common.view_on_map")}
                            className="rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
                          >
                            <MapIcon className="h-3.5 w-3.5" />
                          </button>
                        )
                      }
                    />
                  ))}
                  {addonItems.length > 0 && (
                    <li className="flex items-center gap-2 pt-0.5">
                      {addonItems.length > ADDON_PREVIEW && (
                        <button
                          onClick={() => {
                            openUpdatesModal(true);
                            close();
                          }}
                          className="flex-1 rounded-lg border border-sf-border bg-sf-surface px-3 py-2 text-xs font-medium text-sf-text hover:border-brand-500/40"
                        >
                          {t("notifications.see_all_updates", { n: String(addonItems.length) })}
                        </button>
                      )}
                      {packageCount > 0 && (
                        <button
                          onClick={() => {
                            startUpdateAll();
                            close();
                          }}
                          title={t("notifications.update_all.hint")}
                          className="inline-flex flex-1 items-center justify-center gap-1.5 rounded-lg bg-brand-500 px-3 py-2 text-xs font-semibold text-slate-950 hover:bg-brand-400"
                        >
                          <Sparkles className="h-3.5 w-3.5" />
                          {t("notifications.update_all")} ({packageCount})
                        </button>
                      )}
                    </li>
                  )}
                </Group>
              )}

              {/* Flota y vuelos: mantenimiento y Crew VS. */}
              {fleetCount > 0 && (
                <Group title={t("notifications.group.fleet")} count={fleetCount}>
                  {maintAlerts.map((a) => {
                    const due = a.severity === "due";
                    return (
                      <NoticeCard
                        key={`${a.registration}|${a.component}`}
                        tone={due ? "rose" : "amber"}
                        urgent={due}
                        icon={<Wrench className="h-4 w-4" />}
                        title={
                          due
                            ? t(a.component === "techlog" ? "notifications.mx_nogo" : "notifications.maint_due")
                            : t("notifications.maint_soon")
                        }
                        lines={[
                          <span key="r" className="font-mono text-sf-text/90">
                            {a.registration}
                            {a.model ? ` · ${a.model}` : ""}
                          </span>,
                          `${a.label ?? `${t(`economy.comp.${a.component}`)} · ${Math.round(a.wearPct)}%`}${
                            a.dueCount > 1
                              ? ` · +${a.dueCount - 1} ${t(a.component === "techlog" ? "notifications.mx_more" : "notifications.maint_more")}`
                              : ""
                          }`,
                        ]}
                        onOpen={() => {
                          // (19/10) Avisos del Tech Log → su mantenimiento en el Hangar.
                          if (a.component === "techlog") useAppStore.getState().requestHangarFocus(a.registration);
                          else openEconomy(a.airlineKey, a.registration);
                          close();
                        }}
                        onDismiss={() => maintDismiss(a)}
                      />
                    );
                  })}
                  {duelNotices.map((d) => (
                    <NoticeCard
                      key={d.channel}
                      tone="fuchsia"
                      icon={<Swords className="h-4 w-4" />}
                      title={t("notifications.vs_ready")}
                      lines={[
                        <span key="r" className="font-mono">
                          {d.origin} → {d.dest}
                        </span>,
                        `${t("vs.you")} ${Math.round(d.selfFpm)} · ${d.rivalName} ${Math.round(d.rivalFpm)} fpm`,
                      ]}
                      onOpen={() => {
                        // (v6.2.35) Abre DIRECTO el Crew VS de ese vuelo.
                        requestCrewVs(d.origin, d.dest, d.at);
                        dismissDuelNotice(d.channel);
                        close();
                      }}
                      onDismiss={() => dismissDuelNotice(d.channel)}
                    />
                  ))}
                </Group>
              )}

              {/* Sugerencias: FPS y perfiles GSX ofrecidos. */}
              {suggestCount > 0 && (
                <Group title={t("notifications.group.suggestions")} count={suggestCount}>
                  {fpsNotices.map((n) => (
                    <NoticeCard
                      key={n.folderName}
                      tone="emerald"
                      icon={<Gauge className="h-4 w-4" />}
                      title={n.reason === "updated" ? t("notifications.fps_after_update") : t("notifications.fps_available")}
                      lines={[
                        <span key="t" className="text-sf-text/90">{n.title}</span>,
                        n.reason === "updated" ? t("notifications.fps_after_update_hint") : t("notifications.fps_hint"),
                      ]}
                      onOpen={() => {
                        const pkg = communityPackages.find((p) => p.folderName === n.folderName);
                        if (pkg) {
                          setPerfPkg(pkg);
                          close();
                        } else {
                          // Ya no está instalado — el aviso sobra.
                          fpsDismiss(n.folderName);
                        }
                      }}
                      onDismiss={() => fpsDismiss(n.folderName)}
                    />
                  ))}
                  {gsxPendingOffers.map((o) => (
                    <NoticeCard
                      key={o.icao}
                      tone="cyan"
                      icon={<Truck className="h-4 w-4" />}
                      title={t("gsxoffer.title", { icao: o.icao })}
                      lines={[
                        `${o.airportTitle}${o.profiles.length > 0 ? ` · ${t("gsxoffer.subtitle", { n: String(o.profiles.length) })}` : ""}`,
                      ]}
                      onOpen={() => {
                        reopenGsxOffer(o.icao);
                        close();
                      }}
                      onDismiss={() => clearGsxOffer(o.icao)}
                    />
                  ))}
                </Group>
              )}
            </div>
          </motion.aside>
        )}
      </AnimatePresence>
    </>
  );

  return (
    <div className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        className={cn(
          "relative inline-flex h-8 w-8 items-center justify-center rounded-lg border bg-sf-surface text-sf-text/90 transition-colors hover:border-brand-500/40 hover:text-sf-text",
          open ? "border-brand-500/50 text-sf-text" : "border-sf-border",
        )}
        title={
          totalCount > 0
            ? t("notificationsBell.count_tooltip", { count: String(totalCount) })
            : t("notificationsBell.none_tooltip")
        }
      >
        <Bell className="h-4 w-4" />
        {totalCount > 0 && (
          <span className="absolute -right-1 -top-1 inline-flex h-4 min-w-[1rem] items-center justify-center rounded-full bg-amber-500 px-1 text-[11px] font-bold text-amber-950 ring-2 ring-slate-950">
            {totalCount > 99 ? "99+" : totalCount}
          </span>
        )}
      </button>

      {/* (26/10) Globo al arrancar: cuántos avisos hay, sin abrir el panel. */}
      <AnimatePresence>
        {peek && !open && (
          <motion.button
            initial={{ opacity: 0, y: -4 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={{ duration: 0.15 }}
            onClick={() => setOpen(true)}
            className="absolute right-0 top-full z-30 mt-2 flex w-64 items-center gap-3 rounded-xl border border-sf-border bg-sf-surface px-3 py-2.5 text-left shadow-xl ring-1 ring-slate-950/40 hover:border-brand-500/40"
          >
            <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-amber-500/15 text-amber-300 ring-1 ring-amber-500/25">
              <BellRing className="h-4 w-4" />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-xs font-semibold text-sf-text">
                {peekCount === 1 ? t("notifications.peek_one") : t("notifications.peek", { n: String(peekCount) })}
              </span>
              <span className="block text-[11px] text-sf-muted">{t("notifications.peek_open")}</span>
            </span>
          </motion.button>
        )}
      </AnimatePresence>

      {/* El header tiene backdrop-blur/transform y atrapaba el `fixed`: el
          panel (y el modal de rendimiento) van por portal a <body>. */}
      {createPortal(panel, document.body)}
      {perfPkg &&
        createPortal(
          <PerformanceModal pkg={perfPkg} onClose={() => setPerfPkg(null)} />,
          document.body,
        )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Piezas del panel
// ---------------------------------------------------------------------------

interface AddonItem {
  key: string;
  title: string;
  meta: string;
  image: string | null;
  from: string;
  to: string;
  tooltip: string;
  onOpen: () => void;
  onMap?: () => void;
  onDismiss: () => void;
}

type Tone = "amber" | "emerald" | "cyan" | "indigo" | "fuchsia" | "rose";

const TONES: Record<Tone, { tile: string; stripe: string }> = {
  amber: { tile: "bg-amber-500/15 text-amber-300 ring-amber-500/25", stripe: "before:bg-amber-400" },
  emerald: { tile: "bg-emerald-500/15 text-emerald-300 ring-emerald-500/25", stripe: "before:bg-emerald-400" },
  cyan: { tile: "bg-cyan-500/15 text-cyan-300 ring-cyan-500/25", stripe: "before:bg-cyan-400" },
  indigo: { tile: "bg-indigo-500/15 text-indigo-300 ring-indigo-500/25", stripe: "before:bg-indigo-400" },
  fuchsia: { tile: "bg-fuchsia-500/15 text-fuchsia-300 ring-fuchsia-500/25", stripe: "before:bg-fuchsia-400" },
  rose: { tile: "bg-rose-500/15 text-rose-300 ring-rose-500/25", stripe: "before:bg-rose-400" },
};

function Group({ title, count, children }: { title: string; count: number; children: ReactNode }) {
  return (
    <section>
      <h3 className="mb-2 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-sf-text/80">
        {title}
        <span className="rounded bg-sf-raised px-1.5 py-px text-[10px] tabular-nums text-sf-muted">{count}</span>
      </h3>
      <ul className="space-y-2">{children}</ul>
    </section>
  );
}

/**
 * Un aviso: icono en su cuadrito de color (o la imagen del addon), título,
 * detalle, chip de versión y botones. Clic en el cuerpo = su acción principal.
 */
function NoticeCard({
  tone,
  icon,
  image,
  title,
  lines,
  chip,
  actions,
  extra,
  onOpen,
  openTitle,
  onDismiss,
  urgent,
}: {
  tone: Tone;
  icon: ReactNode;
  image?: string | null;
  title: ReactNode;
  lines?: ReactNode[];
  chip?: ReactNode;
  actions?: ReactNode;
  extra?: ReactNode;
  onOpen?: () => void;
  openTitle?: string;
  onDismiss?: () => void;
  urgent?: boolean;
}) {
  const [imgFailed, setImgFailed] = useState(false);
  const body = (
    <>
      {image && !imgFailed ? (
        <img
          src={image}
          alt=""
          loading="lazy"
          referrerPolicy="no-referrer"
          onError={() => setImgFailed(true)}
          className="h-10 w-14 shrink-0 rounded-lg object-cover ring-1 ring-sf-border"
        />
      ) : (
        <span className={cn("flex h-10 w-10 shrink-0 items-center justify-center rounded-lg ring-1", TONES[tone].tile)}>
          {icon}
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-sf-text">{title}</span>
        {lines?.map((l, i) => (
          <span key={i} className="mt-0.5 block truncate text-[11px] text-sf-muted">
            {l}
          </span>
        ))}
        {chip && <span className="mt-1.5 block">{chip}</span>}
      </span>
    </>
  );
  return (
    <li
      className={cn(
        "group relative overflow-hidden rounded-xl border border-sf-border bg-sf-surface p-3 transition-colors hover:border-slate-500/50",
        urgent && "before:absolute before:inset-y-0 before:left-0 before:w-[3px]",
        urgent && TONES[tone].stripe,
      )}
    >
      <div className="flex items-start gap-3">
        {onOpen ? (
          <button onClick={onOpen} title={openTitle} className="flex min-w-0 flex-1 items-start gap-3 text-left">
            {body}
          </button>
        ) : (
          <div className="flex min-w-0 flex-1 items-start gap-3">{body}</div>
        )}
        {(extra || onDismiss) && (
          <div className="flex shrink-0 items-center gap-0.5">
            {extra}
            {onDismiss && (
              <button
                onClick={onDismiss}
                title={t("common.dismiss")}
                className="rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
              >
                <X className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        )}
      </div>
      {actions && <div className="mt-2.5 flex flex-wrap justify-end gap-1.5">{actions}</div>}
    </li>
  );
}

function VersionChip({ from, to, plain }: { from?: string | null; to?: string | null; plain?: boolean }) {
  // `plain`: números que no son versiones (ciclo AIRAC).
  const v = (s: string) => (!plain && /^\d/.test(s) ? `v${s}` : s);
  if (!from && !to) return null;
  return (
    <span className="inline-flex items-center gap-1 rounded-md bg-sf-raised px-1.5 py-0.5 font-mono text-[10px] text-sf-muted">
      {from && <span>{v(from)}</span>}
      {from && to && <span>→</span>}
      {to && <span className="font-semibold text-amber-300">{v(to)}</span>}
    </span>
  );
}

function PrimaryBtn({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      className="inline-flex items-center gap-1 rounded-md bg-brand-500 px-2.5 py-1 text-[11px] font-semibold text-slate-950 hover:bg-brand-400"
    >
      {children}
    </button>
  );
}

function SecondaryBtn({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      className="inline-flex items-center gap-1 rounded-md border border-sf-border px-2.5 py-1 text-[11px] font-medium text-sf-text hover:border-brand-500/40 hover:bg-sf-raised"
    >
      {children}
    </button>
  );
}

function IconBtn({ title, onClick, children }: { title: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      title={title}
      className="rounded-lg p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
    >
      {children}
    </button>
  );
}
