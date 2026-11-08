import { useEffect, useRef } from "react";
import { PreflightModal } from "./PreflightModal";
import { usePreflightStore } from "../stores/usePreflightStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useToastStore } from "../stores/useToastStore";
import { pickTodayPlan } from "../lib/preflight";
import { usePreflight } from "../lib/usePreflight";
import { t } from "../lib/i18n";

/** Tiempo para que los chequeos del OFP nuevo terminen de cargar (traslado,
 *  GSX, Tech Log…) antes de contar sus puntos pendientes. */
const SETTLE_MS = 4000;

/**
 * (v6.2.34) Anfitrión a nivel App del pre-vuelo:
 *  · Renderiza el modal cuando `usePreflightStore.open`.
 *  · Vigila el OFP del DÍA.
 *
 * (8.4.x) El modal ya NO se abre solo (pedido del usuario): al aterrizar el
 * avión queda en el destino, el chequeo de traslado pasaba a "pendiente" y
 * el modal saltaba encima del sim. Ahora un OFP NUEVO sólo deja un aviso
 * clicable (si tiene puntos por revisar) y queda marcado en cuanto llega,
 * así lo que cambie durante o después del vuelo no vuelve a avisar. El
 * pre-vuelo se abre desde Inicio o la campanita.
 */
export function PreflightHost() {
  const open = usePreflightStore((s) => s.open);
  const setOpen = usePreflightStore((s) => s.setOpen);
  const flights = useSimBriefStore((s) => s.flights);
  const lastAlerted = usePreflightStore((s) => s.lastAlertedOfp);
  const setLastAlerted = usePreflightStore((s) => s.setLastAlertedOfp);
  const { issues } = usePreflight().result;
  const issuesRef = useRef(issues);
  issuesRef.current = issues;
  const timer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (timer.current != null) window.clearTimeout(timer.current);
    },
    [],
  );

  useEffect(() => {
    const plan = pickTodayPlan(flights);
    if (!plan || plan.ofpId === lastAlerted) return;
    setLastAlerted(plan.ofpId);
    if (timer.current != null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      const n = issuesRef.current;
      if (n <= 0) return;
      useToastStore.getState().push({
        kind: "info",
        title: t("preflight.notice.title"),
        message: t("preflight.notice.body", { n: String(n) }),
        onClick: () => setOpen(true),
        ttlMs: 12000,
      });
    }, SETTLE_MS);
  }, [flights, lastAlerted, setLastAlerted, setOpen]);

  return open ? <PreflightModal onClose={() => setOpen(false)} /> : null;
}
