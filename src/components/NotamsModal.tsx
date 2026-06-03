import { useEffect, useState } from "react";
import { motion } from "framer-motion";
import { AlertTriangle, Loader2, X } from "lucide-react";
import type { FlightLogEntry, SimBriefBriefing } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * (v3.32.0 #6) Modal de NOTAMs desde el OFP de SimBrief.
 *
 * Los NOTAMs vienen del briefing on-demand (`simbrief_briefing`): SimBrief
 * los incluye en el OFP SÓLO si el usuario los activa en su layout. Como
 * SimBrief sólo guarda el OFP más reciente, esto es útil para el vuelo que
 * matchea ese OFP (típicamente el recién volado). Si el OFP actual NO
 * matchea origen+destino del vuelo seleccionado, o no trae NOTAMs, se
 * muestra un estado vacío limpio (no inventamos datos).
 */
export function NotamsModal({
  entry,
  onClose,
}: {
  entry: FlightLogEntry;
  onClose: () => void;
}) {
  const [briefing, setBriefing] = useState<SimBriefBriefing | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    api
      .simbriefBriefing()
      .then((b) => {
        if (!cancelled) setBriefing(b);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [entry.id]);

  // ESC cierra.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // ¿El OFP actual de SimBrief corresponde a ESTE vuelo? Comparamos
  // origen+destino (case-insensitive). Sin match no mostramos NOTAMs de
  // otro plan — sería engañoso.
  const norm = (s: string | null | undefined) => (s ?? "").trim().toUpperCase();
  const matches =
    briefing != null &&
    norm(briefing.originIcao) === norm(entry.originIcao) &&
    norm(briefing.destinationIcao) === norm(entry.destinationIcao) &&
    norm(entry.originIcao) !== "";
  const notams = matches ? briefing!.notams : [];

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.12 }}
      onClick={onClose}
      className="absolute inset-0 z-40 flex items-center justify-center bg-slate-950/50 p-4 backdrop-blur-sm"
    >
      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: 8 }}
        transition={{ duration: 0.16 }}
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-full w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-slate-700/80 bg-slate-950/95 shadow-2xl ring-1 ring-slate-800/70 backdrop-blur-xl"
      >
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-slate-800 px-4 py-3">
          <div className="flex items-center gap-2">
            <AlertTriangle className="h-4 w-4 text-rose-300" />
            <h2 className="text-sm font-semibold text-slate-100">
              {t("fb.notams.title")} · {entry.originIcao ?? "?"} → {entry.destinationIcao ?? "?"}
            </h2>
          </div>
          <button
            onClick={onClose}
            title={t("common.close")}
            className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-4">
          {loading ? (
            <div className="flex items-center justify-center gap-2 py-10 text-xs text-slate-400">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("fb.notams.loading")}
            </div>
          ) : error ? (
            <div className="rounded-lg bg-rose-500/10 px-3 py-2 text-xs text-rose-300 ring-1 ring-rose-500/30">
              {error}
            </div>
          ) : !matches ? (
            <EmptyState text={t("fb.notams.empty_no_ofp")} />
          ) : notams.length === 0 ? (
            <EmptyState text={t("fb.notams.empty_none")} />
          ) : (
            <>
              <div className="mb-3 text-[11px] text-slate-500">
                {t("fb.notams.source")}
                {briefing?.generatedAt ? ` · ${briefing.generatedAt}` : ""}
              </div>
              <ul className="space-y-2">
                {notams.map((n, i) => (
                  <li
                    key={i}
                    className="rounded-lg border border-slate-800 bg-slate-900/40 p-3"
                  >
                    {n.location && (
                      <div className="mb-1 inline-block rounded bg-slate-800 px-1.5 py-0.5 font-mono text-[10px] font-semibold text-slate-300">
                        {n.location}
                      </div>
                    )}
                    <pre className="whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-slate-200">
                      {n.text}
                    </pre>
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      </motion.div>
    </motion.div>
  );
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-10 text-center">
      <AlertTriangle className="h-6 w-6 text-slate-600" />
      <p className="max-w-sm text-[11px] leading-relaxed text-slate-500">{text}</p>
    </div>
  );
}
