import { ChevronDown, ChevronUp, Search, X } from "lucide-react";
import { t } from "../lib/i18n";

/**
 * (v4.33.0) Buscador estilo "find in page" del navegador: input +
 * contador `actual/total` + flechas prev/next para navegar entre
 * resultados + botón cerrar. La lógica de "navegar" (scroll a la card
 * en el grid / centrar el nodo en el Link Map) la implementa cada
 * vista vía onPrev/onNext; este componente sólo es la UI.
 */
export function FindSearch({
  value,
  onChange,
  total,
  active,
  onPrev,
  onNext,
  onClose,
  placeholder,
  className,
}: {
  value: string;
  onChange: (v: string) => void;
  /** nº de resultados que matchean. */
  total: number;
  /** índice 1-based del resultado activo (0 = ninguno). */
  active: number;
  onPrev: () => void;
  onNext: () => void;
  onClose: () => void;
  placeholder?: string;
  className?: string;
}) {
  const has = value.trim() !== "";
  return (
    <div
      className={`flex items-center gap-1 rounded-lg border border-sf-border bg-sf-bg/90 pl-2.5 pr-1 py-0.5 shadow-lg backdrop-blur ${className ?? ""}`}
    >
      <Search className="h-3.5 w-3.5 shrink-0 text-sf-muted" />
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            if (e.shiftKey) onPrev();
            else onNext();
          } else if (e.key === "Escape") {
            onClose();
          }
        }}
        placeholder={placeholder ?? t("addons.search.placeholder")}
        className="w-40 bg-transparent py-1 text-xs text-sf-text placeholder:text-sf-muted/70 focus:outline-none xl:w-56"
      />
      {has && (
        <span className="shrink-0 px-1 text-[10px] tabular-nums text-sf-muted">
          {total === 0 ? t("find.none") : `${active}/${total}`}
        </span>
      )}
      <div className="flex items-center">
        <button
          onClick={onPrev}
          disabled={total === 0}
          title={t("find.prev")}
          className="rounded p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text disabled:opacity-40"
        >
          <ChevronUp className="h-3.5 w-3.5" />
        </button>
        <button
          onClick={onNext}
          disabled={total === 0}
          title={t("find.next")}
          className="rounded p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text disabled:opacity-40"
        >
          <ChevronDown className="h-3.5 w-3.5" />
        </button>
        <button
          onClick={onClose}
          title={t("find.close")}
          className="rounded p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
    </div>
  );
}
