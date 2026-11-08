import { useState } from "react";
import { Star } from "lucide-react";
import { getActiveLocale, t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { ADDON_SORTS, type AddonSort } from "../lib/addonSort";
import { useCommunityStore } from "../stores/useCommunityStore";

/** Fecha corta de la última actualización de un addon. */
export function fmtUpdate(sqlite: string | null | undefined): string | null {
  if (!sqlite) return null;
  const d = new Date(`${sqlite.replace(" ", "T")}Z`);
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString(getActiveLocale(), { day: "numeric", month: "short", year: "numeric" });
}

/** (04/11) Etapa 9 — estrellas de sólo lectura, para tarjetas y listas. */
export function StarsShown({ value, className }: { value: number | null | undefined; className?: string }) {
  if (!value) return null;
  return (
    <span
      role="img"
      aria-label={t("rating.shown", { n: value })}
      title={t("rating.shown", { n: value })}
      className={cn("inline-flex shrink-0 items-center gap-px", className)}
    >
      {[1, 2, 3, 4, 5].map((n) => (
        <Star key={n} className={cn("h-3 w-3", n <= value ? "fill-amber-400 text-amber-400" : "text-sf-muted/40")} />
      ))}
    </span>
  );
}

/** Estrellas que se pueden cambiar. Pulsar la marcada quita la valoración. */
export function StarsPicker({ folder, value }: { folder: string; value: number | null | undefined }) {
  const [hover, setHover] = useState(0);
  const shown = hover || value || 0;
  return (
    <span
      role="radiogroup"
      aria-label={t("rating.label")}
      className="inline-flex items-center"
      onMouseLeave={() => setHover(0)}
    >
      {[1, 2, 3, 4, 5].map((n) => {
        const label = n === value ? t("rating.clear") : t("rating.set", { n });
        return (
          <button
            key={n}
            type="button"
            role="radio"
            aria-checked={n === value}
            aria-label={label}
            title={label}
            onMouseEnter={() => setHover(n)}
            onClick={(e) => {
              e.stopPropagation();
              void useCommunityStore.getState().setRating(folder, n === value ? null : n);
            }}
            className="rounded p-0.5 hover:bg-sf-raised"
          >
            <Star className={cn("h-4 w-4", n <= shown ? "fill-amber-400 text-amber-400" : "text-sf-muted/60")} />
          </button>
        );
      })}
    </span>
  );
}

/** Selector «Ordenar» de Instalados y del Mapa de escenarios. */
/** (06/11) Un filtro o un agrupado con su nombre delante. */
export function LabeledSelect({
  label,
  value,
  options,
  onChange,
  className,
}: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  className?: string;
}) {
  return (
    <label className={cn("inline-flex min-w-0 items-center gap-1.5 text-xs text-sf-muted", className)}>
      <span className="shrink-0">{label}</span>
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="min-w-0 max-w-44 flex-1 rounded-lg border border-sf-border bg-sf-surface px-2 py-1 text-xs text-sf-text"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </label>
  );
}

export function AddonSortSelect({
  value,
  onChange,
  className,
}: {
  value: AddonSort;
  onChange: (v: AddonSort) => void;
  className?: string;
}) {
  return (
    <label className={cn("inline-flex shrink-0 items-center gap-1.5 text-xs text-sf-muted", className)}>
      {t("addons.sort.label")}
      <select
        value={value}
        onChange={(e) => onChange(e.target.value as AddonSort)}
        className="rounded-lg border border-sf-border bg-sf-surface px-2 py-1 text-xs text-sf-text"
      >
        {ADDON_SORTS.map((s) => (
          <option key={s} value={s}>
            {t(`addons.sort.${s}`)}
          </option>
        ))}
      </select>
    </label>
  );
}
