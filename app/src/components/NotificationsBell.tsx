import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  Bell,
  CheckCheck,
  Download,
  ExternalLink,
  Map as MapIcon,
  RefreshCw,
  Sparkles,
  X,
} from "lucide-react";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useAppStore } from "../stores/useAppStore";
import type { AvailableUpdate, UpdateInfo } from "../lib/types";
import { api } from "../lib/tauri";
import { deriveUpdateSearchQuery } from "../lib/updateSearchQuery";

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

  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const [appUpdate, setAppUpdate] = useState<UpdateInfo | null>(null);
  const [appUpdateDismissed, setAppUpdateDismissed] = useState(false);

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
  const packageCount = updates.length;
  const totalCount = packageCount + (showAppUpdate ? 1 : 0);

  // Peek-on-first-launch: si hay notificaciones y no hemos peekado
  // antes, abrimos brevemente para que el usuario las vea.
  useEffect(() => {
    if (totalCount === 0) return;
    const PEEK_KEY = "msfs-addons:notifications-peeked";
    if (typeof window === "undefined") return;
    if (window.sessionStorage.getItem(PEEK_KEY)) return;
    window.sessionStorage.setItem(PEEK_KEY, "1");
    setOpen(true);
    const t = window.setTimeout(() => setOpen(false), 4000);
    return () => window.clearTimeout(t);
  }, [totalCount]);

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
                  Notificaciones
                </h3>
              </div>
              <div className="flex items-center gap-1.5">
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
                  title="Volver a comprobar updates ahora"
                  className="rounded-md border border-slate-800 p-1.5 text-slate-300 hover:border-brand-500/40 hover:bg-slate-900"
                >
                  <RefreshCw className="h-3.5 w-3.5" />
                </button>
                {packageCount > 0 && (
                  <button
                    onClick={() => dismissAll()}
                    title="Marcar todas las updates de paquetes como vistas"
                    className="rounded-md border border-slate-800 p-1.5 text-slate-300 hover:border-brand-500/40 hover:bg-slate-900"
                  >
                    <CheckCheck className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>
            </header>

            {/* App update — pinned al top, en verde para diferenciar
                de los updates de paquetes (ámbar). */}
            {showAppUpdate && appUpdate && (
              <div className="border-b border-emerald-500/20 bg-emerald-500/10 px-4 py-3">
                <div className="flex items-start gap-3">
                  <Sparkles className="mt-0.5 h-4 w-4 shrink-0 text-emerald-300" />
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-semibold text-emerald-100">
                      Nueva versión de la app
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
                          <Download className="h-3 w-3" /> Descargar
                        </button>
                      ) : (
                        <button
                          onClick={() => api.openExternal(appUpdate.releaseUrl)}
                          className="inline-flex items-center gap-1 rounded-md bg-emerald-500/30 px-2.5 py-1 text-[11px] font-medium text-emerald-100 hover:bg-emerald-500/40"
                        >
                          <ExternalLink className="h-3 w-3" /> Ver release
                        </button>
                      )}
                    </div>
                  </div>
                  <button
                    onClick={dismissAppUpdate}
                    title="No volver a avisarme sobre esta versión"
                    className="shrink-0 rounded-md p-1 text-emerald-200/70 hover:bg-emerald-500/20 hover:text-emerald-100"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            )}

            {packageCount > 0 && (
              <div className="border-b border-slate-800 bg-amber-500/5 px-4 py-2">
                <button
                  onClick={handleUpdateAll}
                  className="inline-flex w-full items-center justify-center gap-2 rounded-md bg-amber-500 px-3 py-2 text-xs font-bold uppercase tracking-wide text-amber-950 shadow-md shadow-amber-500/20 hover:bg-amber-400"
                >
                  <Sparkles className="h-3.5 w-3.5" />
                  Actualizar todo ({packageCount})
                </button>
                <p className="mt-1 text-center text-[10px] text-slate-500">
                  Te preguntaremos el método de descarga para cada uno.
                </p>
              </div>
            )}

            <div className="max-h-[60vh] overflow-y-auto">
              {lastRefreshError && (
                <div className="m-3 rounded bg-rose-500/15 px-3 py-2 text-xs text-rose-300 ring-1 ring-rose-500/30">
                  {lastRefreshError}
                </div>
              )}
              {totalCount === 0 ? (
                <div className="px-4 py-8 text-center text-xs text-slate-500">
                  Todo al día. La app comprueba updates automáticamente al
                  arrancar y al ganar foco.
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
                          title="Ver en el mapa"
                          className="rounded-md border border-slate-800 p-1 text-slate-400 hover:border-brand-500/40 hover:text-slate-100"
                        >
                          <MapIcon className="h-3.5 w-3.5" />
                        </button>
                        <button
                          onClick={() => dismissUpdate(u.folderName)}
                          title="Descartar"
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
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
