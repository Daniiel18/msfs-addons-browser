import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  Check,
  ChevronRight,
  PlaneLanding,
  X,
} from "lucide-react";
import type { SimBriefFlight } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useToastStore } from "../stores/useToastStore";

/**
 * (v4.17.0) Modal de confirmación de OFP al APAGADO DE MOTORES — solución
 * para cuentas SimBrief COMPARTIDAS, donde la API devuelve el último plan
 * generado por cualquiera de los usuarios y el auto-link cruzaba callsigns.
 *
 * Flujo:
 *   1. El watcher (populate_simbrief_async con confirm_mode=true) emite
 *      `simbrief://confirm` con los OFP candidatos del caché local
 *      (`simbrief_flights`), mejor puntuado primero. NO auto-linkea nada.
 *   2. Este modal muestra el candidato actual: "¿Es este tu vuelo?" con
 *      callsign, origen → destino y aeronave/matrícula.
 *   3. "NO" → carrusel: avanza al siguiente OFP más reciente del caché
 *      (en memoria — cero llamadas de red), con wrap-around.
 *   4. "SÍ" → `link_simbrief_ofp` (backend) hace la DOBLE VALIDACIÓN por
 *      matrícula: match → guarda; OFP sin matrícula → bypass y guarda;
 *      mismatch → devuelve `regMismatch` y aquí se muestra la alerta de
 *      conflicto exigiendo la última confirmación (reintento force=true).
 *
 * El vuelo en sí (telemetría/FPM) ya está guardado por finish_flight — lo
 * que queda "pendiente" hasta el SÍ es el enlace SimBrief (callsign /
 * flight number / airline), que es lo que se cruzaba entre usuarios.
 */

interface ConfirmPayload {
  flightId: number;
  candidates: SimBriefFlight[];
}

type Step = "ask" | "mismatch";

export function SimBriefConfirmModal() {
  const [payload, setPayload] = useState<ConfirmPayload | null>(null);
  const [idx, setIdx] = useState(0);
  const [step, setStep] = useState<Step>("ask");
  const [busy, setBusy] = useState(false);
  const [mismatch, setMismatch] = useState<{
    simbriefReg: string | null;
    simReg: string | null;
  } | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    listen<ConfirmPayload>("simbrief://confirm", (event) => {
      if (!event.payload?.candidates?.length) return;
      setPayload(event.payload);
      setIdx(0);
      setStep("ask");
      setMismatch(null);
      setBusy(false);
    })
      .then((u) => {
        unlisten = u;
      })
      .catch(() => {});
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  if (!payload) return null;
  const candidates = payload.candidates;
  const cur = candidates[idx % candidates.length];
  const label = cur.callsign || cur.flightNumber || cur.ofpId;

  const close = () => {
    setPayload(null);
    setMismatch(null);
    setStep("ask");
  };

  const onNo = () => {
    // Carrusel: siguiente OFP más reciente del caché, con wrap-around.
    setIdx((i) => (i + 1) % candidates.length);
    setStep("ask");
    setMismatch(null);
  };

  const onYes = async (force: boolean) => {
    setBusy(true);
    try {
      const outcome = await api.linkSimbriefOfp(
        payload.flightId,
        cur.ofpId,
        force,
      );
      if (outcome.status === "regMismatch") {
        setMismatch({
          simbriefReg: outcome.simbriefReg,
          simReg: outcome.simReg,
        });
        setStep("mismatch");
        setBusy(false);
        return;
      }
      useToastStore.getState().push({
        kind: "success",
        title: t("sbconfirm.toast.linked"),
        message: `${label} · ${cur.originIcao} → ${cur.destinationIcao}`,
      });
      close();
    } catch (e) {
      useToastStore.getState().push({
        kind: "error",
        title: t("sbconfirm.toast.error"),
        message: String(e),
      });
      setBusy(false);
    }
  };

  return (
    <AnimatePresence>
      <motion.div
        key="sb-confirm"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        className="fixed inset-0 z-[70] flex items-center justify-center bg-slate-950/70 px-4 backdrop-blur-sm"
      >
        <motion.div
          initial={{ y: 14, opacity: 0 }}
          animate={{ y: 0, opacity: 1 }}
          exit={{ y: 14, opacity: 0 }}
          transition={{ duration: 0.16 }}
          className="w-full max-w-md rounded-2xl border border-slate-700 bg-slate-950 p-5 shadow-2xl ring-1 ring-slate-800/70"
        >
          {/* Header */}
          <div className="flex items-start justify-between gap-3">
            <div className="flex items-center gap-2">
              <PlaneLanding className="h-4 w-4 text-emerald-300" />
              <h2 className="text-sm font-semibold text-slate-100">
                {t("sbconfirm.title")}
              </h2>
            </div>
            <button
              onClick={close}
              disabled={busy}
              title={t("common.close")}
              className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-40"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          {step === "ask" ? (
            <>
              <p className="mt-2 text-xs text-slate-400">
                {t("sbconfirm.question")}
              </p>
              {/* Card del candidato actual */}
              <div className="mt-3 rounded-xl border border-slate-800 bg-slate-900/60 px-4 py-3">
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono text-lg font-bold text-slate-100">
                    {label}
                  </span>
                  <span className="rounded bg-slate-800 px-1.5 py-0.5 font-mono text-[10px] text-slate-400">
                    {(idx % candidates.length) + 1} / {candidates.length}
                  </span>
                </div>
                <div className="mt-1 font-mono text-sm text-sky-200">
                  {cur.originIcao} → {cur.destinationIcao}
                </div>
                <div className="mt-1 text-[11px] text-slate-400">
                  {cur.aircraftIcao ?? "—"}
                  {cur.aircraftReg ? (
                    <span className="ml-2 font-mono text-slate-300">
                      {cur.aircraftReg}
                    </span>
                  ) : null}
                </div>
              </div>
              <div className="mt-4 flex items-center justify-end gap-2">
                <button
                  onClick={onNo}
                  disabled={busy}
                  className="inline-flex items-center gap-1 rounded-md border border-slate-700 bg-slate-900/60 px-4 py-2 text-xs font-medium text-slate-300 hover:border-slate-600 hover:text-slate-100 disabled:opacity-40"
                >
                  {t("sbconfirm.no")}
                  <ChevronRight className="h-3.5 w-3.5" />
                </button>
                <button
                  onClick={() => void onYes(false)}
                  disabled={busy}
                  className="inline-flex items-center gap-1 rounded-md bg-emerald-500/85 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-emerald-400 disabled:opacity-40"
                >
                  <Check className="h-3.5 w-3.5" />
                  {t("sbconfirm.yes")}
                </button>
              </div>
            </>
          ) : (
            <>
              {/* Alerta de conflicto de matrícula (doble validación) */}
              <div className="mt-3 rounded-xl border border-amber-500/40 bg-amber-500/10 px-4 py-3">
                <div className="flex items-center gap-2 text-xs font-semibold text-amber-300">
                  <AlertTriangle className="h-4 w-4" />
                  {t("sbconfirm.mismatch.title")}
                </div>
                <p className="mt-1.5 text-[11px] leading-relaxed text-amber-100/90">
                  {t("sbconfirm.mismatch.body")}
                </p>
                <div className="mt-2 grid grid-cols-2 gap-2 font-mono text-[11px]">
                  <div className="rounded bg-slate-900/60 px-2 py-1.5">
                    <div className="text-[9px] uppercase tracking-wide text-slate-500">
                      SimBrief
                    </div>
                    <div className="text-slate-200">
                      {mismatch?.simbriefReg ?? "—"}
                    </div>
                  </div>
                  <div className="rounded bg-slate-900/60 px-2 py-1.5">
                    <div className="text-[9px] uppercase tracking-wide text-slate-500">
                      {t("sbconfirm.mismatch.sim")}
                    </div>
                    <div className="text-slate-200">
                      {mismatch?.simReg ?? "—"}
                    </div>
                  </div>
                </div>
              </div>
              <div className="mt-4 flex items-center justify-end gap-2">
                <button
                  onClick={() => {
                    setStep("ask");
                    setMismatch(null);
                  }}
                  disabled={busy}
                  className="rounded-md border border-slate-700 bg-slate-900/60 px-4 py-2 text-xs font-medium text-slate-300 hover:border-slate-600 hover:text-slate-100 disabled:opacity-40"
                >
                  {t("sbconfirm.mismatch.back")}
                </button>
                <button
                  onClick={() => void onYes(true)}
                  disabled={busy}
                  className="inline-flex items-center gap-1 rounded-md bg-amber-500/85 px-4 py-2 text-xs font-semibold text-slate-950 hover:bg-amber-400 disabled:opacity-40"
                >
                  <Check className="h-3.5 w-3.5" />
                  {t("sbconfirm.mismatch.confirm")}
                </button>
              </div>
            </>
          )}
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
