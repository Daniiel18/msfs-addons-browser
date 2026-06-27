import { useEffect } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, Loader2, X } from "lucide-react";
import { useGsxUpdateStore } from "../stores/useGsxUpdateStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { t } from "../lib/i18n";

/**
 * (v6.2.6) Gate de actualización de GSX. GSX/couatl no puede actualizarse con el
 * simulador abierto (los archivos están en uso). Cuando el usuario pide
 * actualizar y el sim está corriendo, mostramos este warning y NO abrimos el
 * FSDT Installer hasta detectar el sim cerrado. En cuanto `simRunning` pasa a
 * false, lanzamos la instalación (abre el instalador + borra la notificación).
 *
 * Se monta una vez en App (headless salvo cuando hay un install pendiente).
 */
export function GsxUpdateGate() {
  const pendingInstall = useGsxUpdateStore((s) => s.pendingInstall);
  const performInstall = useGsxUpdateStore((s) => s.performInstall);
  const cancelInstall = useGsxUpdateStore((s) => s.cancelInstall);
  // `simRunning` se refresca por el evento `flight://current` (~5s al cerrar).
  const simRunning = useFlightLogStore((s) => s.status?.simRunning ?? false);

  useEffect(() => {
    // Sim cerrado mientras esperábamos → procede: abre instalador + limpia.
    if (pendingInstall && !simRunning) {
      void performInstall();
    }
  }, [pendingInstall, simRunning, performInstall]);

  return (
    <AnimatePresence>
      {pendingInstall && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          className="fixed inset-0 z-[80] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
        >
          <motion.div
            initial={{ scale: 0.95, y: 10 }}
            animate={{ scale: 1, y: 0 }}
            exit={{ scale: 0.95, y: 10 }}
            className="w-[min(440px,calc(100vw-2rem))] overflow-hidden rounded-2xl border border-amber-500/40 bg-slate-950 shadow-2xl ring-1 ring-amber-500/20"
          >
            <div className="flex items-start gap-3 border-b border-amber-500/20 bg-amber-500/10 px-5 py-4">
              <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-amber-300" />
              <div className="min-w-0 flex-1">
                <h3 className="text-sm font-semibold text-amber-100">
                  {t("gsx.gate.title")}
                </h3>
                <p className="mt-1 text-xs leading-relaxed text-amber-200/80">
                  {t("gsx.gate.body")}
                </p>
              </div>
              <button
                onClick={cancelInstall}
                title={t("common.cancel")}
                className="shrink-0 rounded-md p-1 text-amber-200/70 hover:bg-amber-500/20 hover:text-amber-100"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="flex items-center gap-2 px-5 py-4 text-sm text-slate-300">
              <Loader2 className="h-4 w-4 animate-spin text-amber-300" />
              {t("gsx.gate.waiting")}
            </div>
            <div className="flex justify-end border-t border-slate-800 px-5 py-3">
              <button
                onClick={cancelInstall}
                className="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
              >
                {t("common.cancel")}
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
