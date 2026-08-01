import { useEffect, useMemo } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Loader2, Trophy, X } from "lucide-react";
import { useAchievementsStore } from "../stores/useAchievementsStore";
import { MedalBadge, TIERS } from "./MedalBadge";
import { t } from "../lib/i18n";
import type { AchievementProgress } from "../lib/types";

/** Orden canónico de las secciones. */
const CATEGORIES = [
  "flights",
  "landings",
  "exploration",
  "aircraft",
  "airline",
  "milestones",
] as const;

/** Formatea el valor de una métrica (las horas y las millas se ven mejor
 *  redondeadas; el resto son conteos enteros). */
function fmt(id: string, v: number): string {
  const n = id === "hours_total" || id === "hours_on_type" ? Math.round(v * 10) / 10 : Math.round(v);
  return n.toLocaleString();
}

function AchievementCard({ a }: { a: AchievementProgress }) {
  const locked = !a.unlocked;
  const maxed = a.nextThreshold == null && a.unlocked;
  const pct = maxed
    ? 100
    : a.nextThreshold
      ? Math.min(100, Math.round((a.current / a.nextThreshold) * 100))
      : 0;

  return (
    <div
      className={`flex items-center gap-3 rounded-xl border border-slate-800 bg-slate-900/50 p-3 transition-colors hover:border-slate-700 ${
        locked ? "opacity-55" : ""
      }`}
    >
      <MedalBadge icon={a.icon} tier={a.tier} size={58} />
      <div className="min-w-0 flex-1">
        <div className="text-[13px] font-semibold text-slate-100">
          {t(`ach.${a.id}.name`)}
        </div>
        <div className="mt-0.5 line-clamp-2 text-[11px] text-slate-500">
          {t(`ach.${a.id}.desc`)}
        </div>
        <div className="mt-2 h-[5px] overflow-hidden rounded-full bg-slate-800">
          <div
            className="h-full rounded-full bg-gradient-to-r from-sky-400 to-violet-400"
            style={{ width: `${pct}%` }}
          />
        </div>
        <div className="mt-1 flex items-center justify-between text-[10px] text-slate-500">
          <span className="tabular-nums">{fmt(a.id, a.current)}</span>
          <span>
            {maxed
              ? t("ach.maxed")
              : `→ ${fmt(a.id, a.nextThreshold ?? 0)}`}
          </span>
        </div>
        {/* pips de niveles conseguidos */}
        <div className="mt-1.5 flex gap-[3px]">
          {a.tiers.map((_, i) => (
            <span
              key={i}
              className="h-1 w-3 rounded-full"
              style={{
                backgroundColor:
                  a.tier != null && i <= a.tier
                    ? TIERS[Math.min(i, TIERS.length - 1)].ring
                    : "#1e293b",
              }}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

export function AchievementsModal({ onClose }: { onClose: () => void }) {
  const list = useAchievementsStore((s) => s.list);
  const loading = useAchievementsStore((s) => s.loading);
  const ensureFresh = useAchievementsStore((s) => s.ensureFresh);

  useEffect(() => {
    ensureFresh();
  }, [ensureFresh]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const { unlocked, byCategory } = useMemo(() => {
    const byCategory = new Map<string, AchievementProgress[]>();
    for (const a of list) {
      const arr = byCategory.get(a.category) ?? [];
      arr.push(a);
      byCategory.set(a.category, arr);
    }
    return { unlocked: list.filter((a) => a.unlocked).length, byCategory };
  }, [list]);

  return createPortal(
    <AnimatePresence>
      <motion.div
        className="fixed inset-0 z-[80] flex items-center justify-center bg-black/60 p-4"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        onClick={onClose}
      >
        <motion.div
          initial={{ opacity: 0, scale: 0.97, y: 8 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.97, y: 8 }}
          transition={{ duration: 0.15 }}
          onClick={(e) => e.stopPropagation()}
          className="flex max-h-[88vh] w-full max-w-4xl flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
        >
          <header className="flex items-center justify-between gap-3 border-b border-slate-800 px-5 py-4">
            <div className="flex items-center gap-3">
              <Trophy className="h-5 w-5 text-amber-300" />
              <div>
                <h2 className="text-sm font-semibold text-slate-100">
                  {t("ach.title")}
                </h2>
                <p className="mt-0.5 text-xs text-slate-400">
                  {t("ach.summary", {
                    n: String(unlocked),
                    total: String(list.length),
                  })}
                </p>
              </div>
            </div>
            <div className="flex items-center gap-2">
              {loading && (
                <Loader2 className="h-4 w-4 animate-spin text-slate-500" />
              )}
              <button
                onClick={onClose}
                title={t("common.dismiss")}
                className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          </header>

          <div className="min-h-0 flex-1 overflow-y-auto p-4">
            {list.length === 0 ? (
              <div className="flex flex-col items-center justify-center gap-2 py-16 text-center">
                {loading ? (
                  <Loader2 className="h-6 w-6 animate-spin text-slate-600" />
                ) : (
                  <>
                    <Trophy className="h-8 w-8 text-slate-700" />
                    <p className="text-sm text-slate-400">{t("ach.empty")}</p>
                  </>
                )}
              </div>
            ) : (
              CATEGORIES.map((cat) => {
                const items = byCategory.get(cat) ?? [];
                if (items.length === 0) return null;
                const got = items.filter((a) => a.unlocked).length;
                return (
                  <section key={cat} className="mb-5 last:mb-0">
                    <h3 className="mb-2 flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
                      {t(`ach.cat.${cat}`)}
                      <span className="rounded-full bg-slate-800 px-2 py-0.5 text-[10px] font-medium normal-case tracking-normal text-slate-500">
                        {got}/{items.length}
                      </span>
                    </h3>
                    <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
                      {items.map((a) => (
                        <AchievementCard key={a.id} a={a} />
                      ))}
                    </div>
                  </section>
                );
              })
            )}
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>,
    document.body,
  );
}
