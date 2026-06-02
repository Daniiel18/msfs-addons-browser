/**
 * (v4.0.0 P7.7) Banner top-right que aparece cuando el watcher detecta
 * modo Replay o Slew activo en el simulador.
 *
 * Backend emite `flight://replay` con payload:
 *   { active: bool, simRate: number, isSlew: bool }
 *
 * Mientras está activo, el watcher CONGELA toda la lógica del state
 * machine — no persiste track, no transiciona OOOI, no escribe FPM.
 * Esto evita corromper la DB con telemetría re-vivida cuando el piloto
 * usa el replay para revisar su propio aterrizaje, o cuando se mueve
 * libremente con Slew.
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { AnimatePresence, motion } from "framer-motion";
import { PauseCircle } from "lucide-react";
import { useEffect, useState } from "react";

import { t } from "../lib/i18n";

interface ReplayPayload {
  active: boolean;
  simRate: number;
  isSlew: boolean;
  isExternalReplay: boolean;
  /** "external_replay" | "slew" | "time_regress" | "sim_rate" | "normal" */
  cause: string;
}

export function ReplayBanner() {
  const [state, setState] = useState<ReplayPayload | null>(null);

  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    listen<ReplayPayload>("flight://replay", (event) => {
      if (event.payload.active) {
        setState(event.payload);
      } else {
        setState(null);
      }
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((e) => {
        // eslint-disable-next-line no-console
        console.error("[ReplayBanner] listen failed", e);
      });
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  return (
    <AnimatePresence>
      {state && (
        <motion.div
          initial={{ opacity: 0, y: -20 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -20 }}
          transition={{ duration: 0.18 }}
          className="fixed left-1/2 top-3 z-[85] -translate-x-1/2 transform rounded-full border border-rose-400/40 bg-rose-950/90 px-4 py-1.5 shadow-2xl backdrop-blur"
        >
          <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-rose-200">
            <PauseCircle size={14} className="text-rose-300" />
            <span>{t("flight.replay.banner")}</span>
            <span className="rounded bg-rose-900/80 px-1.5 py-0.5 font-mono text-[10px] text-rose-100">
              {state.isExternalReplay
                ? "REPLAY"
                : state.isSlew
                  ? "SLEW"
                  : state.cause === "time_regress"
                    ? "TIME"
                    : `${state.simRate.toFixed(2)}×`}
            </span>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
