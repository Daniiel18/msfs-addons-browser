/**
 * (v4.0.0 P7.5b) Toast de confirmación manual cuando el scorer de
 * SimBrief detecta 2+ candidatos viables para un vuelo en curso.
 *
 * Backend emite `simbrief://ambiguous` con payload:
 *   { flight_id: number, candidates: ScoredOfp[] }
 *
 * Cada candidato trae score + breakdown (factor → puntos). El usuario
 * elige uno → llamada a `linkSimbriefOfp` → el vuelo queda linkeado.
 *
 * Diseño: bottom-right, glass, animado con framer-motion. Dismiss
 * implícito si el usuario cierra; el OFP queda sin atribuir hasta
 * que se cierre el vuelo (puede editarse manualmente con "Edit").
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { AnimatePresence, motion } from "framer-motion";
import { CheckCircle2, X } from "lucide-react";
import { useEffect, useState } from "react";

import { t } from "../lib/i18n";
import { api } from "../lib/tauri";

type ScoreBreakdown = [string, number];

interface OfpCandidate {
  ofpId: string;
  pilotId: string;
  flightNumber: string | null;
  callsign: string | null;
  aircraftIcao: string | null;
  aircraftReg: string | null;
  originIcao: string;
  destinationIcao: string;
  generatedAt: string | null;
}

interface ScoredOfp {
  ofp: OfpCandidate;
  score: number;
  breakdown: ScoreBreakdown[];
}

interface AmbiguousPayload {
  flightId: number;
  candidates: ScoredOfp[];
}

export function SimBriefAmbiguityToast() {
  const [payload, setPayload] = useState<AmbiguousPayload | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<AmbiguousPayload>("simbrief://ambiguous", (event) => {
      setPayload(event.payload);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((e) => {
        // eslint-disable-next-line no-console
        console.error("[SimBriefAmbiguityToast] listen failed", e);
      });
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const handleSelect = async (ofpId: string) => {
    if (!payload || busy) return;
    setBusy(true);
    try {
      await api.linkSimbriefOfp(payload.flightId, ofpId);
      setPayload(null);
    } catch (e) {
      // eslint-disable-next-line no-console
      console.error("[SimBriefAmbiguityToast] linkSimbriefOfp failed", e);
    } finally {
      setBusy(false);
    }
  };

  const handleDismiss = () => {
    if (busy) return;
    setPayload(null);
  };

  return (
    <AnimatePresence>
      {payload && (
        <motion.div
          initial={{ opacity: 0, y: 30, scale: 0.95 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: 30, scale: 0.95 }}
          transition={{ duration: 0.18 }}
          className="fixed bottom-6 right-6 z-[80] w-[360px] rounded-2xl border border-amber-300/30 bg-slate-900/95 p-4 shadow-2xl backdrop-blur"
        >
          <div className="flex items-start justify-between gap-2">
            <div>
              <div className="text-xs font-semibold uppercase tracking-wide text-amber-300">
                {t("simbrief.ambiguity.title")}
              </div>
              <div className="mt-1 text-sm text-slate-200">
                {t("simbrief.ambiguity.body")}
              </div>
            </div>
            <button
              onClick={handleDismiss}
              disabled={busy}
              aria-label={t("simbrief.ambiguity.dismiss")}
              className="rounded-lg p-1 text-slate-500 transition hover:bg-slate-800 hover:text-slate-300 disabled:opacity-50"
            >
              <X size={16} />
            </button>
          </div>

          <ul className="mt-3 space-y-2">
            {payload.candidates.map((c) => (
              <li key={c.ofp.ofpId}>
                <button
                  onClick={() => handleSelect(c.ofp.ofpId)}
                  disabled={busy}
                  className="group flex w-full items-center justify-between gap-2 rounded-xl border border-slate-700/60 bg-slate-800/60 px-3 py-2 text-left transition hover:border-amber-400/60 hover:bg-slate-800/90 disabled:opacity-60"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 text-sm font-semibold text-slate-100">
                      <span>{c.ofp.callsign ?? c.ofp.flightNumber ?? "—"}</span>
                      {c.ofp.aircraftReg && (
                        <span className="rounded bg-slate-700/80 px-1.5 py-0.5 text-[10px] font-mono text-amber-200">
                          {c.ofp.aircraftReg}
                        </span>
                      )}
                    </div>
                    <div className="mt-0.5 text-xs text-slate-400">
                      {c.ofp.originIcao} → {c.ofp.destinationIcao}
                      {c.ofp.aircraftIcao && ` · ${c.ofp.aircraftIcao}`}
                    </div>
                    <div className="mt-1 flex flex-wrap gap-1 text-[10px] text-slate-500">
                      {c.breakdown
                        .filter(([, pts]) => pts !== 0)
                        .map(([factor, pts]) => (
                          <span
                            key={factor}
                            className={
                              pts > 0
                                ? "rounded bg-emerald-900/50 px-1.5 py-0.5 text-emerald-300"
                                : "rounded bg-rose-900/50 px-1.5 py-0.5 text-rose-300"
                            }
                          >
                            {factor} {pts > 0 ? "+" : ""}
                            {pts}
                          </span>
                        ))}
                    </div>
                  </div>
                  <div className="flex flex-col items-end gap-1">
                    <span className="text-base font-bold text-amber-300">
                      {c.score}
                    </span>
                    <CheckCircle2
                      size={16}
                      className="text-slate-600 transition group-hover:text-amber-400"
                    />
                  </div>
                </button>
              </li>
            ))}
          </ul>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
