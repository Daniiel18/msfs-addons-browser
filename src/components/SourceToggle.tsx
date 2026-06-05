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
      className="no-select relative inline-flex items-center rounded-full border border-slate-800 bg-slate-900/70 p-1 backdrop-blur"
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
              "relative z-10 rounded-full px-5 py-1.5 text-sm font-medium transition-colors",
              active ? "text-slate-950" : "text-slate-300 hover:text-white"
            )}
          >
            {active && (
              <motion.span
                layoutId="source-toggle-pill"
                transition={{ type: "spring", stiffness: 500, damping: 40 }}
                className="absolute inset-0 -z-10 rounded-full bg-brand-400 shadow-[0_0_20px_rgba(63,191,120,0.45)]"
              />
            )}
            {s.name}
          </button>
        );
      })}
    </div>
  );
}
