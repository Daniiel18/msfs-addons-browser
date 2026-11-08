import { useEffect, useState } from "react";
import {
  CheckCircle2,
  Circle,
  CircleCheck,
  CircleSlash,
  Clock,
  Loader2,
  MinusCircle,
  Rocket,
  X,
  XCircle,
} from "lucide-react";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { launchError } from "../lib/launchFlow";
import type { LaunchRunStep } from "../lib/types";
import { useLaunchStore } from "../stores/useLaunchStore";

/** Al terminar, la tarjeta se va sola a los 15 s. */
const AUTO_HIDE_MS = 15_000;

/**
 * (02/11) Etapa 6 — tarjeta de la secuencia en marcha (abajo al centro, lejos
 * de los avisos): pasos con su estado, cuenta atrás, «Seguir ya» y Cancelar.
 */
export function LaunchProgressCard() {
  const run = useLaunchStore((s) => s.run);
  // Sólo redibuja la cuenta atrás; la hora se lee al pintar.
  const [, setTick] = useState(0);
  const [hover, setHover] = useState(false);

  const counting = run?.state === "running" && run.steps.some((s) => s.status === "waiting" && s.waitUntilMs);
  useEffect(() => {
    if (!counting) return;
    const id = window.setInterval(() => setTick((n) => n + 1), 500);
    return () => window.clearInterval(id);
  }, [counting]);

  const finished = !!run && run.state !== "running";
  useEffect(() => {
    // Si un script detuvo el arranque, el motivo se queda hasta que lo cierres.
    if (!finished || hover || run?.state === "stopped") return;
    const id = window.setTimeout(() => useLaunchStore.getState().dismissRun(), AUTO_HIDE_MS);
    return () => window.clearTimeout(id);
  }, [finished, hover, run?.runId]);

  if (!run) return null;
  const total = run.steps.length;
  const done = run.steps.filter((s) => !["pending", "waiting", "opening"].includes(s.status)).length;
  const failed = run.steps.filter((s) => s.status === "failed").length;

  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      className="fixed bottom-4 left-1/2 z-[66] w-[min(400px,calc(100vw-2rem))] -translate-x-1/2 overflow-hidden rounded-2xl border border-sf-border bg-sf-bg shadow-2xl ring-1 ring-sf-border"
    >
      <div className="flex items-center gap-2.5 px-4 pb-2 pt-3">
        <Rocket className="h-4 w-4 shrink-0 text-brand-400" />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-semibold text-sf-text">
            {run.state === "running"
              ? run.name
              : run.state === "cancelled"
                ? t("launch.run.cancelled_title")
                : run.state === "stopped"
                  ? t("launch.run.stopped_title")
                  : t("launch.run.done")}
            {run.testAll && (
              <span className="ml-2 rounded bg-sf-raised px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-sf-muted">
                {t("launch.run.title_test")}
              </span>
            )}
          </p>
          <p className="text-[11px] text-sf-muted">
            {run.state === "running" ? t("launch.run.progress", { done: String(done), total: String(total) }) : run.name}
          </p>
        </div>
        {run.state === "running" ? (
          <button
            type="button"
            onClick={() => void useLaunchStore.getState().cancel()}
            className="rounded-lg border border-sf-border px-2.5 py-1 text-xs font-medium text-sf-text hover:bg-sf-raised"
          >
            {t("launch.run.cancel")}
          </button>
        ) : (
          <button
            type="button"
            onClick={() => useLaunchStore.getState().dismissRun()}
            aria-label={t("launch.run.dismiss")}
            className="rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </div>
      <div className="mx-4 h-1 overflow-hidden rounded-full bg-sf-raised">
        <div
          className={cn("h-full rounded-full transition-all", failed > 0 ? "bg-amber-400" : "bg-emerald-400")}
          style={{ width: `${total > 0 ? Math.round((done / total) * 100) : 0}%` }}
        />
      </div>
      <ul className="max-h-56 space-y-1 overflow-y-auto px-4 py-2.5">
        {run.steps.map((s) => (
          <StepLine key={s.id} step={s} />
        ))}
      </ul>
      {run.restartSim && (
        <p className="border-t border-sf-border px-4 py-2 text-[11px] font-medium leading-snug text-amber-300">
          {t("dispatch.restart_sim")}
        </p>
      )}
      {run.closing.length > 0 && (
        <p className="border-t border-sf-border px-4 py-2 text-[11px] leading-snug text-sf-muted">
          {t("launch.run.closing", { names: run.closing.join(", ") })}
        </p>
      )}
    </div>
  );
}

function StepLine({ step }: { step: LaunchRunStep }) {
  const now = Date.now();
  const icon = {
    pending: <Circle className="h-3.5 w-3.5 text-sf-muted/60" />,
    waiting: <Clock className="h-3.5 w-3.5 text-sky-400" />,
    opening: <Loader2 className="h-3.5 w-3.5 animate-spin text-sky-400" />,
    opened: <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400" />,
    already: <CircleCheck className="h-3.5 w-3.5 text-sf-muted" />,
    skipped: <MinusCircle className="h-3.5 w-3.5 text-sf-muted" />,
    failed: <XCircle className="h-3.5 w-3.5 text-rose-400" />,
    cancelled: <CircleSlash className="h-3.5 w-3.5 text-sf-muted/60" />,
  }[step.status];

  let note: string | null = null;
  if (step.status === "waiting") {
    note =
      step.wait === "menu"
        ? t("launch.run.wait_menu")
        : step.wait === "loaded"
          ? t("launch.run.wait_loaded")
          : t("launch.run.wait_seconds", {
              n: String(Math.max(0, Math.ceil(((step.waitUntilMs ?? now) - now) / 1000))),
            });
  } else if (step.status === "opening") {
    note = t(step.kind === "script" ? "launch.run.running_script" : "launch.run.opening");
  }
  else if (step.status === "already") note = t("launch.run.already");
  else if (step.status === "skipped") note = t("launch.run.skipped");
  else if (step.status === "cancelled") note = t("launch.run.cancelled");
  else if (step.status === "failed") note = launchError(step.error);
  if (step.kind === "addons") {
    const a = step.addons;
    if (step.status === "opening") note = t("launch.run.addons_working");
    else if (a && a.failed > 0) note = t("dispatch.failed", { n: a.failed });
    else if (a) note = t("dispatch.applied", { on: a.on, off: a.off });
  }

  return (
    <li className="flex items-center gap-2 text-xs">
      <span className="shrink-0">{icon}</span>
      <span
        className={cn(
          "min-w-0 flex-1 truncate",
          ["pending", "cancelled"].includes(step.status) ? "text-sf-muted" : "text-sf-text",
        )}
      >
        {step.name}
        {note && (
          <span className={cn("ml-1.5", step.status === "failed" ? "text-rose-300" : "text-sf-muted")}>· {note}</span>
        )}
      </span>
      {step.status === "waiting" && (
        <button
          type="button"
          onClick={() => void useLaunchStore.getState().skipWait()}
          className="shrink-0 rounded-md px-1.5 py-0.5 text-[11px] font-semibold text-brand-300 hover:bg-sf-raised"
        >
          {t("launch.run.skip")}
        </button>
      )}
    </li>
  );
}
