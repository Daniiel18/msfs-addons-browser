import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import {
  Bell,
  CheckCheck,
  Download,
  Loader2,
  Map as MapIcon,
  RefreshCw,
  Sparkles,
  X,
} from "lucide-react";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useAppStore } from "../stores/useAppStore";

/**
 * Campana de notificaciones del header.
 *
 * Cambios respecto a la versión con localStorage:
 *   · El estado "dismissed" vive ahora en backend (tabla
 *     `dismissed_updates`). Eso sobrevive a cambios de máquina y,
 *     más importante, **se limpia al pulsar "Recargar"** para que
 *     las pendientes vuelvan siempre a aparecer.
 *   · Botón "Actualizar todo" que llena la cola del wizard global
 *     (`UpdateWizard`) y deja al usuario elegir método uno por uno.
 */
export function NotificationsBell() {
  const updates = useCommunityStore((s) => s.updates);
  const refreshUpdates = useCommunityStore((s) => s.refreshUpdates);
  const refreshing = useCommunityStore((s) => s.refreshing);
  const lastRefreshError = useCommunityStore((s) => s.lastRefreshError);
  const setFocused = useCommunityStore((s) => s.setFocused);
  const dismissUpdate = useCommunityStore((s) => s.dismissUpdate);
  const dismissAll = useCommunityStore((s) => s.dismissAll);
  const startUpdateAll = useCommunityStore((s) => s.startUpdateAll);
  const setView = useAppStore((s) => s.setView);
  const triggerSearch = useAppStore((s) => s.triggerSearch);

  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement | null>(null);

  // Cierre al click fuera
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (!wrapRef.current?.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [open]);

  const count = updates.length;

  // Peek-on-first-launch: si esta sesión es la primera apertura
  // y hay updates, abrimos brevemente el dropdown para que el
  // usuario las vea. Se cierra solo a los 4 s. El flag persiste en
  // sessionStorage para que recargas (HMR) no re-disparen el peek.
  useEffect(() => {
    if (count === 0) return;
    const PEEK_KEY = "msfs-addons:notifications-peeked";
    if (typeof window === "undefined") return;
    if (window.sessionStorage.getItem(PEEK_KEY)) return;
    window.sessionStorage.setItem(PEEK_KEY, "1");
    setOpen(true);
    const t = window.setTimeout(() => setOpen(false), 4000);
    return () => window.clearTimeout(t);
  }, [count]);

  const focusOnMap = (folderName: string) => {
    setView("map");
    setFocused(folderName);
    setOpen(false);
  };

  const openInSearch = (icao: string, source: string) => {
    triggerSearch(icao, source);
    setOpen(false);
  };

  const handleUpdateAll = () => {
    startUpdateAll();
    setOpen(false);
  };

  return (
    <div ref={wrapRef} className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        className="relative inline-flex h-9 w-9 items-center justify-center rounded-lg border border-slate-800 bg-slate-900/60 text-slate-300 transition-colors hover:border-brand-500/40 hover:text-slate-100"
        title={count > 0 ? `${count} actualización(es) disponibles` : "Sin actualizaciones"}
      >
        <Bell className="h-4 w-4" />
        {count > 0 && (
          <span className="absolute -right-1 -top-1 inline-flex h-4 min-w-[1rem] items-center justify-center rounded-full bg-amber-500 px-1 text-[10px] font-bold text-amber-950 ring-2 ring-slate-950">
            {count > 99 ? "99+" : count}
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
            className="absolute right-0 top-full z-30 mt-2 w-[400px] origin-top-right overflow-hidden rounded-xl border border-slate-800 bg-slate-950/95 shadow-xl ring-1 ring-slate-800 backdrop-blur"
          >
            <header className="flex items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
              <div>
                <h3 className="text-sm font-semibold text-slate-100">
                  Actualizaciones
                </h3>
                <p className="text-[11px] text-slate-500">
                  Comparado con el catálogo de fuentes activas
                </p>
              </div>
              <div className="flex items-center gap-1">
                {count > 0 && (
                  <button
                    onClick={() => dismissAll()}
                    title="Marcar todas como vistas (volverán al recargar)"
                    className="rounded-md border border-slate-800 p-1.5 text-slate-300 hover:border-brand-500/40 hover:bg-slate-900"
                  >
                    <CheckCheck className="h-3.5 w-3.5" />
                  </button>
                )}
                <button
                  onClick={() => refreshUpdates()}
                  disabled={refreshing}
                  title="Buscar actualizaciones ahora (vuelve a mostrar las descartadas)"
                  className="rounded-md border border-slate-800 p-1.5 text-slate-300 hover:border-brand-500/40 hover:bg-slate-900 disabled:opacity-50"
                >
                  {refreshing ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <RefreshCw className="h-3.5 w-3.5" />
                  )}
                </button>
              </div>
            </header>

            {count > 0 && (
              <div className="border-b border-slate-800 bg-amber-500/5 px-4 py-2">
                <button
                  onClick={handleUpdateAll}
                  className="inline-flex w-full items-center justify-center gap-2 rounded-md bg-amber-500 px-3 py-2 text-xs font-bold uppercase tracking-wide text-amber-950 shadow-md shadow-amber-500/20 hover:bg-amber-400"
                >
                  <Sparkles className="h-3.5 w-3.5" />
                  Actualizar todo ({count})
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
              {count === 0 ? (
                <div className="px-4 py-8 text-center text-xs text-slate-500">
                  {refreshing
                    ? "Comprobando actualizaciones…"
                    : "Todo al día. Pulsa el botón para volver a comprobar."}
                </div>
              ) : (
                <ul className="divide-y divide-slate-800">
                  {updates.map((u) => (
                    <li key={u.folderName} className="group flex items-stretch hover:bg-slate-900/60">
                      <button
                        onClick={() => openInSearch(u.icao, u.source)}
                        className="flex flex-1 items-start gap-3 px-4 py-3 text-left"
                        title={`Buscar ${u.icao} en ${u.source} para descargar la nueva versión`}
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
                          title="Descartar (vuelve al recargar)"
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
