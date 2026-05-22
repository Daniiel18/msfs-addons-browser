import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Download, Loader2, Sparkles, X } from "lucide-react";
import { api } from "../lib/tauri";
import type { UpdateInfo } from "../lib/types";

/**
 * (v2.1.0) Badge "Hay update" en el header — entre el título y los
 * controles de ventana. Aparece cuando la app está abierta y el
 * polling periódico detecta una versión más nueva.
 *
 * Click → modal con changelog + botón "Instalar ahora" + checkbox
 * "Reiniciar al terminar".
 */
export function LiveUpdateBadge() {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [showModal, setShowModal] = useState(false);
  const [autoRestart, setAutoRestart] = useState(true);
  const [installing, setInstalling] = useState(false);
  const [downloadPct, setDownloadPct] = useState<number | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);

  // (v2.2.0) Polling cada 1 hora — antes era 30 min y los logs del
  // usuario se llenaban con requests constantes. La primera
  // verificación se hace al montar (con 5s de delay para no chocar
  // con el chequeo del bootstrap). La búsqueda manual desde la bell
  // sigue siendo inmediata.
  useEffect(() => {
    let cancelled = false;
    const check = async () => {
      try {
        const u = await api.checkForUpdate();
        if (cancelled) return;
        if (u && u.latestVersion !== dismissed) {
          setUpdate(u);
        } else if (!u) {
          setUpdate(null);
        }
      } catch (e) {
        console.warn("LiveUpdateBadge: check falló:", e);
      }
    };
    const t1 = setTimeout(check, 5_000);
    const interval = setInterval(check, 60 * 60_000); // 1 h
    return () => {
      cancelled = true;
      clearTimeout(t1);
      clearInterval(interval);
    };
  }, [dismissed]);

  const onInstall = async () => {
    if (!update?.assetUrl) return;
    setInstalling(true);
    setDownloadPct(0);
    try {
      const unsub = await api.onUpdateProgress((p) => {
        if (p.totalBytes && p.totalBytes > 0) {
          setDownloadPct(
            Math.min(100, Math.round((p.downloadedBytes / p.totalBytes) * 100)),
          );
        }
      });
      try {
        await api.installUpdate(update.assetUrl, autoRestart);
        // Si llegamos aquí sin que la app se haya cerrado, mostramos
        // mensaje. Normalmente la app exit(0) y el usuario no ve esto.
      } finally {
        unsub();
      }
    } catch (e) {
      console.error("install failed:", e);
      setInstalling(false);
      setDownloadPct(null);
    }
  };

  if (!update) return null;

  return (
    <>
      <button
        onClick={() => setShowModal(true)}
        className="pointer-events-auto inline-flex items-center gap-1 rounded-full border border-emerald-500/40 bg-emerald-500/15 px-2 py-0.5 text-[10px] font-semibold text-emerald-200 shadow-lg shadow-emerald-500/10 hover:bg-emerald-500/25"
        title={`v${update.latestVersion} disponible — click para ver detalles`}
      >
        <motion.span
          animate={{ scale: [1, 1.2, 1] }}
          transition={{ duration: 2, repeat: Infinity }}
          className="inline-block h-1.5 w-1.5 rounded-full bg-emerald-400"
        />
        <Sparkles className="h-2.5 w-2.5" />
        v{update.latestVersion}
      </button>

      <AnimatePresence>
        {showModal && (
          <motion.div
            className="fixed inset-0 z-[70] flex items-start justify-center overflow-y-auto bg-slate-950/80 px-4 py-10 backdrop-blur-sm"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={() => !installing && setShowModal(false)}
          >
            <motion.div
              initial={{ y: 16, opacity: 0 }}
              animate={{ y: 0, opacity: 1 }}
              exit={{ y: 16, opacity: 0 }}
              transition={{ duration: 0.18 }}
              onClick={(e) => e.stopPropagation()}
              className="w-full max-w-md rounded-2xl border border-emerald-500/30 bg-slate-950 shadow-2xl"
            >
              <header className="flex items-center justify-between border-b border-slate-800 px-5 py-3">
                <div className="flex items-center gap-2 text-sm font-semibold text-emerald-200">
                  <Sparkles className="h-4 w-4 text-emerald-300" />
                  Actualización disponible
                </div>
                <button
                  onClick={() => !installing && setShowModal(false)}
                  className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-50"
                  disabled={installing}
                >
                  <X className="h-4 w-4" />
                </button>
              </header>

              <div className="space-y-3 p-5">
                <div className="rounded-md border border-emerald-500/20 bg-emerald-500/5 px-3 py-2">
                  <div className="font-mono text-sm text-emerald-100">
                    v{update.currentVersion} → v{update.latestVersion}
                  </div>
                  <a
                    href={update.releaseUrl}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="mt-0.5 inline-block text-[10px] text-emerald-300 hover:underline"
                  >
                    Ver release en GitHub →
                  </a>
                </div>

                {update.notesMarkdown && (
                  <div>
                    <div className="mb-1 text-[11px] uppercase tracking-wider text-slate-400">
                      Cambios
                    </div>
                    <div className="max-h-48 overflow-y-auto whitespace-pre-wrap rounded-md border border-slate-800 bg-slate-900/50 px-3 py-2 font-mono text-[11px] leading-relaxed text-slate-300">
                      {update.notesMarkdown.slice(0, 8000)}
                      {update.notesMarkdown.length > 8000 && "…"}
                    </div>
                  </div>
                )}

                {installing && (
                  <div className="rounded-md border border-emerald-500/30 bg-emerald-500/5 px-3 py-2 text-xs text-emerald-200">
                    <div className="mb-1 flex items-center gap-2">
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      Descargando…
                      {downloadPct !== null && (
                        <span className="ml-auto font-mono">
                          {downloadPct}%
                        </span>
                      )}
                    </div>
                    {downloadPct !== null && (
                      <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
                        <div
                          className="h-full bg-emerald-400 transition-all duration-200"
                          style={{ width: `${downloadPct}%` }}
                        />
                      </div>
                    )}
                    <p className="mt-1.5 text-[10px] text-emerald-200/70">
                      La app se cerrará tras la descarga.{" "}
                      {autoRestart
                        ? "Se reiniciará automáticamente al terminar."
                        : "Tendrás que abrirla manualmente."}
                    </p>
                  </div>
                )}

                <label className="flex cursor-pointer items-center gap-2 text-xs text-slate-300">
                  <input
                    type="checkbox"
                    checked={autoRestart}
                    onChange={(e) => setAutoRestart(e.target.checked)}
                    disabled={installing}
                    className="accent-emerald-500"
                  />
                  Reiniciar la app automáticamente al terminar
                </label>
              </div>

              <footer className="flex items-center justify-end gap-2 border-t border-slate-800 bg-slate-900/40 px-5 py-3">
                <button
                  onClick={() => {
                    setDismissed(update.latestVersion);
                    setShowModal(false);
                  }}
                  disabled={installing}
                  className="rounded-md border border-slate-800 px-3 py-1.5 text-xs text-slate-300 hover:border-slate-700 disabled:opacity-50"
                >
                  Recordar más tarde
                </button>
                <button
                  onClick={onInstall}
                  disabled={installing || !update.assetUrl}
                  className="inline-flex items-center gap-1 rounded-md bg-emerald-500 px-3 py-1.5 text-xs font-semibold text-emerald-950 hover:bg-emerald-400 disabled:opacity-40"
                >
                  {installing ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Download className="h-3.5 w-3.5" />
                  )}
                  Instalar ahora
                </button>
              </footer>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>
    </>
  );
}
