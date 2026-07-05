import { useEffect } from "react";
import { PreflightModal } from "./PreflightModal";
import { usePreflightStore } from "../stores/usePreflightStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { pickTodayPlan } from "../lib/preflight";
import { playPreflightChime } from "../lib/sound";

/**
 * (v6.2.34) Anfitrión a nivel App del pre-vuelo:
 *  · Renderiza el modal cuando `usePreflightStore.open`.
 *  · Vigila el OFP del DÍA: al detectar uno nuevo (id distinto del último
 *    avisado), suena una campana, trae la app al frente y abre el modal
 *    para que el usuario vea qué falta antes de volar.
 */
async function focusApp() {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const win = getCurrentWindow();
    await win.unminimize().catch(() => {});
    await win.setFocus().catch(() => {});
  } catch {
    /* fuera de Tauri (demo) — no-op */
  }
}

export function PreflightHost() {
  const open = usePreflightStore((s) => s.open);
  const setOpen = usePreflightStore((s) => s.setOpen);
  const flights = useSimBriefStore((s) => s.flights);
  const lastAlerted = usePreflightStore((s) => s.lastAlertedOfp);
  const setLastAlerted = usePreflightStore((s) => s.setLastAlertedOfp);

  useEffect(() => {
    const plan = pickTodayPlan(flights);
    if (plan && plan.ofpId !== lastAlerted) {
      setLastAlerted(plan.ofpId);
      playPreflightChime();
      void focusApp();
      setOpen(true);
    }
  }, [flights, lastAlerted, setLastAlerted, setOpen]);

  return open ? <PreflightModal onClose={() => setOpen(false)} /> : null;
}
