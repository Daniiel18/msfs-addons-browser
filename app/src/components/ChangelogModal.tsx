import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { AlertCircle, ExternalLink, FileText, Loader2, X } from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

interface Props {
  /** URL de la página de detalle del addon (Simplaza/SceneryAddons).
   *  El backend scrapea el HTML buscando un heading "Changelog" /
   *  "What's new" / "Release notes" y devuelve las líneas. */
  pageUrl: string;
  /** Título a mostrar en el header — normalmente `addon.name`. */
  title: string;
  /** Etiqueta de la fuente para mostrar al lado del título.
   *  Opcional — útil para diferenciar Simplaza de SceneryAddons. */
  sourceLabel?: string;
  /** Versión del addon a mostrar al lado del título. Opcional. */
  version?: string | null;
  onClose: () => void;
}

/**
 * Modal flotante con el changelog scrapeado de la página de detalle
 * del addon. Se carga lazy al abrir: el efecto dispara `fetchChangelog`
 * y reflexiona el estado en spinner / error / lista.
 *
 * Lo invoca el botón "Changelog" en `ResultCard.tsx` (sólo para
 * resultados de Simplaza — SceneryAddons no expone changelog útil).
 * Antes vivía como sección desplegable dentro de `PackageDetailModal`,
 * pero el usuario pidió moverlo a los cards de búsqueda y dejarlo
 * fuera del detalle de paquete instalado.
 */
export function ChangelogModal({
  pageUrl,
  title,
  sourceLabel,
  version,
  onClose,
}: Props) {
  const [loading, setLoading] = useState(true);
  const [lines, setLines] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setLines(null);
    api
      .fetchChangelog(pageUrl)
      .then((cl) => {
        if (cancelled) return;
        setLines(cl.lines);
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [pageUrl]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <AnimatePresence>
      <motion.div
        key="changelog-backdrop"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        onClick={onClose}
        className="fixed inset-0 z-[60] flex items-center justify-center bg-slate-950/70 backdrop-blur-sm"
      >
        <motion.div
          key="changelog-modal"
          initial={{ opacity: 0, scale: 0.96, y: 8 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.96, y: 8 }}
          transition={{ duration: 0.16 }}
          onClick={(e) => e.stopPropagation()}
          className="relative w-[min(600px,calc(100vw-2rem))] max-h-[calc(100vh-3rem)] overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
        >
          <header className="flex items-start justify-between gap-3 border-b border-slate-800 px-5 py-3">
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-xs text-slate-400">
                <FileText className="h-3.5 w-3.5 text-brand-300" />
                <span>Changelog</span>
                {sourceLabel && (
                  <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-slate-400">
                    {sourceLabel}
                  </span>
                )}
                {version && (
                  <span className="font-mono text-[10px] text-slate-500">
                    v{version}
                  </span>
                )}
              </div>
              <h2 className="mt-1 truncate text-sm font-semibold text-slate-100">
                {title}
              </h2>
            </div>
            <div className="flex shrink-0 items-center gap-1">
              <button
                onClick={() => api.openExternal(pageUrl)}
                title={t("changelog.open_source")}
                className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
              >
                <ExternalLink className="h-4 w-4" />
              </button>
              <button
                onClick={onClose}
                className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          </header>

          <div className="overflow-y-auto px-5 py-4 text-xs">
            {loading && (
              <div className="inline-flex items-center gap-2 text-slate-400">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                {t("changelog.loading")}
              </div>
            )}
            {!loading && error && (
              <div className="flex items-start gap-2 rounded-md border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-rose-200">
                <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>{error}</span>
              </div>
            )}
            {!loading && !error && lines && lines.length === 0 && (
              <p className="text-slate-500">
                {t("changelog.not_found")}
              </p>
            )}
            {!loading && !error && lines && lines.length > 0 && (
              <ul className="ml-3 list-disc space-y-1 text-slate-300">
                {lines.slice(0, 80).map((l, i) => (
                  <li key={i} className="leading-snug">
                    {l}
                  </li>
                ))}
                {lines.length > 80 && (
                  <li className="italic text-slate-500">
                    {t("changelog.more_lines", { count: String(lines.length - 80) })}
                  </li>
                )}
              </ul>
            )}
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
