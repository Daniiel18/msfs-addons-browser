import { RotateCcw } from "lucide-react";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import type { DispatchFlight } from "../lib/dispatch";
import type { DispatchMode } from "../lib/types";
import { useDispatchPlan, useDispatchStore, type LaunchPlan } from "../stores/useDispatchStore";

export const DISPATCH_MODES: Exclude<DispatchMode, "set">[] = ["just", "add", "focus"];

/** Lo que la fila del avión dice de la livery en En vuelo. */
export const liveryNote = (plan: LaunchPlan): string | null =>
  plan.livery ? t(`inflight.need.livery_${plan.livery.status}`) : null;

/** La línea que cuenta qué va a cambiar. */
export function dispatchLine(plan: LaunchPlan): { text: string; warn: boolean } {
  if (plan.needsChoice) return { text: t("dispatch.line.choose"), warn: true };
  if (plan.noFlight) return { text: t("dispatch.line.no_flight"), warn: false };
  if (plan.mode === "just") return { text: t("dispatch.line.just"), warn: false };
  const prefix = plan.set
    ? `${t("dispatch.line.set", { name: plan.set.name })} · `
    : plan.trip
      ? `${t("dispatch.line.trip", { n: plan.trip.stops.length })} · `
      : "";
  const on = plan.on.length;
  const off = plan.off.length;
  const counts =
    on > 0 && off > 0
      ? t("dispatch.line.both", { on, off })
      : on > 0
        ? t("dispatch.line.on", { n: on })
        : off > 0
          ? t("dispatch.line.off", { n: off })
          : t(plan.set ? "dispatch.line.set_same" : "dispatch.line.ready");
  return { text: `${prefix}${counts}`, warn: false };
}

/**
 * (03/11) Etapa 7 — modo de lanzamiento encima de «Lanzar» en En vuelo. La línea
 * de debajo cuenta los cambios y abre el detalle.
 */
export function DispatchSelector({ flight, className }: { flight: DispatchFlight | null; className?: string }) {
  const mode = useDispatchStore((s) => s.mode);
  const pending = useDispatchStore((s) => s.pendingRestore);
  const plan = useDispatchPlan(flight);
  const line = dispatchLine(plan);
  const hasDetail = plan.needsChoice || plan.on.length + plan.off.length + plan.kept.length > 0;

  return (
    <div className={className}>
      <div
        role="radiogroup"
        aria-label={t("dispatch.selector")}
        className="grid grid-cols-3 gap-0.5 rounded-lg bg-sf-raised p-0.5"
      >
        {DISPATCH_MODES.map((m) => (
          <button
            key={m}
            type="button"
            role="radio"
            aria-checked={mode === m}
            title={`${t(`dispatch.mode.${m}`)} · ${t(`dispatch.mode.${m}_hint`)}`}
            onClick={() => void useDispatchStore.getState().setMode(m)}
            className={cn(
              "truncate rounded-md px-1.5 py-1 text-[11px] font-medium transition-colors",
              mode === m ? "bg-sf-surface text-sf-text shadow-sm" : "text-sf-muted hover:text-sf-text",
            )}
          >
            {t(`dispatch.seg.${m}`)}
          </button>
        ))}
      </div>
      <button
        type="button"
        disabled={!hasDetail}
        onClick={() => useDispatchStore.getState().openReview({ kind: "flight", flight: plan.flight, seqId: null })}
        title={line.text}
        className={cn(
          "mt-1 block w-full truncate text-left text-[11px]",
          line.warn ? "font-medium text-amber-300" : "text-sf-muted",
          hasDetail && "hover:text-sf-text hover:underline",
        )}
      >
        {line.text}
      </button>
      {pending !== null && (
        <button
          type="button"
          onClick={() => void useDispatchStore.getState().restore()}
          title={t("dispatch.pending.hint")}
          className="mt-0.5 inline-flex max-w-full items-center gap-1 truncate text-[11px] font-medium text-brand-300 hover:underline"
        >
          <RotateCcw className="h-3 w-3 shrink-0" />
          {t("dispatch.pending.restore")}
        </button>
      )}
    </div>
  );
}
