import { Layers } from "lucide-react";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { useActiveSet, useDispatchStore } from "../stores/useDispatchStore";

/** (06/11) Conjunto activo y, si ya no coincide, cómo guardar lo que hay. */
export function ActiveSetBadge({ className }: { className?: string }) {
  const active = useActiveSet();
  if (!active) return null;
  const { set, changed } = active;
  const store = useDispatchStore.getState();
  return (
    <div className={cn("flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px]", className)}>
      <button
        type="button"
        onClick={() => store.openSets()}
        title={t("dispatch.active.label")}
        className="inline-flex min-w-0 items-center gap-1 text-sf-muted hover:text-sf-text"
      >
        <Layers className="h-3 w-3 shrink-0" />
        <span className="truncate">{set.name}</span>
      </button>
      {changed && (
        <>
          <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-amber-300">
            {t("dispatch.active.changed")}
          </span>
          <button
            type="button"
            onClick={() => void store.updateSet(set.id)}
            className="font-medium text-brand-300 hover:underline"
          >
            {t("dispatch.active.save")}
          </button>
          <button type="button" onClick={() => store.openSets()} className="font-medium text-sf-muted hover:text-sf-text hover:underline">
            {t("dispatch.active.save_new")}
          </button>
        </>
      )}
    </div>
  );
}
