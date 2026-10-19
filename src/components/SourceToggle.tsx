import { motion } from "framer-motion";
import { cn } from "../lib/cn";
import type { SourceDescriptor } from "../lib/types";
import { t } from "../lib/i18n";

interface Props {
  sources: SourceDescriptor[];
  activeId: string;
  onChange: (id: string) => void;
}

export function SourceToggle({ sources, activeId, onChange }: Props) {
  if (sources.length === 0) return null;

  return (
    <div
      role="tablist"
      aria-label={t("source.aria")}
      className="no-select relative inline-flex h-9 items-center rounded-full border border-sf-border bg-sf-surface p-1"
    >
      {sources.map((s) => {
        const active = s.id === activeId;
        return (
          <button
            key={s.id}
            role="tab"
            aria-selected={active}
            onClick={() => onChange(s.id)}
            className={cn(
              "relative z-10 rounded-full px-4 py-1 text-[13px] font-medium transition-colors",
              active ? "text-slate-950" : "text-sf-muted hover:text-sf-text"
            )}
          >
            {active && (
              <motion.span
                layoutId="source-toggle-pill"
                transition={{ type: "spring", stiffness: 500, damping: 40 }}
                className="absolute inset-0 -z-10 rounded-full bg-brand-400 shadow-[0_0_20px_rgba(53,205,178,0.45)]"
              />
            )}
            {s.name}
          </button>
        );
      })}
    </div>
  );
}
