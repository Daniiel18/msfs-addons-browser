import { useState } from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, Download, RefreshCw, X } from "lucide-react";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { t } from "../lib/i18n";
import type { DownloadMethod } from "../lib/types";

/**
 * (v7.1) Modal de recuperación tras un fallo de descarga por TORRENT (sin seeds
 * o timeout). Se abre solo (lo dispara `useDownloadsStore.reconcile`) y reofrece
 * los mirrors del addon para que el usuario no quede en un callejón sin salida.
 * El botón de torrent aparece EN GRIS; al clickearlo explica por qué. Abajo hay
 * un "Reintentar torrent". Los mirrors arrancan el flujo normal.
 */
export function DownloadRecoveryModal() {
  const recovery = useDownloadsStore((s) => s.recovery);
  const dismiss = useDownloadsStore((s) => s.dismissRecovery);
  const start = useDownloadsStore((s) => s.start);
  const [showWhy, setShowWhy] = useState(false);

  if (!recovery) return null;
  const { addonId, addonTitle, source, addonSimulator, methods, reason } = recovery;

  const pick = (method: DownloadMethod) => {
    void start({ addonId, addonTitle, source, method, addonSimulator, allMethods: methods });
    dismiss();
  };

  const torrent = methods.find((m) => m.kind === "torrent");
  const mirrors = methods.filter((m) => m.kind !== "torrent");

  return createPortal(
    <div
      className="fixed inset-0 z-[75] flex items-center justify-center bg-black/60 p-4"
      onClick={dismiss}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        className="w-full max-w-md rounded-2xl border border-slate-800 bg-slate-900 p-6 shadow-2xl ring-1 ring-slate-800"
      >
        <div className="flex items-start justify-between">
          <div className="flex items-center gap-2 text-amber-300">
            <AlertTriangle className="h-5 w-5" />
            <h2 className="text-base font-semibold text-slate-100">
              {t("dl.recover.title")}
            </h2>
          </div>
          <button
            onClick={dismiss}
            aria-label={t("common.dismiss")}
            className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <p className="mt-2 text-sm text-slate-400">
          {t("dl.recover.body", { title: addonTitle })}
        </p>

        <div className="mt-4 flex flex-col gap-2">
          {torrent && (
            <button
              onClick={() => setShowWhy((v) => !v)}
              title={t("dl.recover.why")}
              className="inline-flex cursor-help items-center justify-between gap-2 rounded-lg border border-slate-800 bg-slate-800/40 px-3 py-2 text-sm text-slate-500"
            >
              <span className="inline-flex items-center gap-2">
                <Download className="h-4 w-4" />
                {torrent.name}
              </span>
              <span className="text-[11px] uppercase tracking-wide text-slate-600">
                {t("dl.recover.failed")}
              </span>
            </button>
          )}
          {showWhy && (
            <p className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs leading-relaxed text-amber-200">
              {t("dl.recover.why")}
              {reason ? <span className="text-amber-300/70"> ({reason})</span> : null}
            </p>
          )}

          {mirrors.map((m, i) => (
            <button
              key={i}
              onClick={() => pick(m)}
              className="inline-flex items-center gap-2 rounded-lg border border-slate-700 bg-slate-800/80 px-3 py-2 text-sm font-medium text-slate-100 transition-colors hover:border-brand-500/60 hover:bg-slate-800"
            >
              <Download className="h-4 w-4" />
              {m.name}
            </button>
          ))}
        </div>

        {torrent && (
          <button
            onClick={() => pick(torrent)}
            className="mt-4 inline-flex w-full items-center justify-center gap-2 rounded-lg border border-brand-500/40 bg-brand-500/10 px-3 py-2 text-sm font-medium text-brand-200 transition-colors hover:bg-brand-500/20"
          >
            <RefreshCw className="h-4 w-4" />
            {t("dl.recover.retry")}
          </button>
        )}
      </div>
    </div>,
    document.body,
  );
}
