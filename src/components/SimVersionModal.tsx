import { useState } from "react";
import { motion } from "framer-motion";
import { Check, Loader2, Plane } from "lucide-react";
import { useSettingsStore } from "../stores/useSettingsStore";
import { t } from "../lib/i18n";

/**
 * (v5.1.0) Modal de elección de versión de MSFS al primer arranque.
 * Bloqueante: el usuario elige 2020 o 2024 y la app recarga para
 * aplicar (filtro del catálogo de SceneryAddons + carpeta Community de
 * esa versión). Sólo aparece cuando `settings.simVersion` está vacío.
 */
export function SimVersionModal() {
  const setSimVersion = useSettingsStore((s) => s.setSimVersion);
  const [busy, setBusy] = useState<"msfs2020" | "msfs2024" | null>(null);

  const choose = async (v: "msfs2020" | "msfs2024") => {
    if (busy) return;
    setBusy(v);
    try {
      await setSimVersion(v);
      // Recarga para reaplicar Community + filtro del catálogo.
      window.location.reload();
    } catch {
      setBusy(null);
    }
  };

  return (
    <div className="fixed inset-0 z-[120] flex items-center justify-center bg-slate-950/85 p-4 backdrop-blur-sm">
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.18 }}
        className="w-[min(520px,calc(100vw-2rem))] overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
      >
        <header className="flex items-center gap-2.5 border-b border-slate-800 px-5 py-4">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-brand-500/15 ring-1 ring-brand-500/40">
            <Plane className="h-4 w-4 text-brand-300" />
          </div>
          <div>
            <h2 className="text-sm font-semibold text-slate-100">
              {t("simver.title")}
            </h2>
            <p className="text-[11px] text-slate-500">{t("simver.subtitle")}</p>
          </div>
        </header>

        <div className="grid grid-cols-1 gap-3 p-5 sm:grid-cols-2">
          {(["msfs2020", "msfs2024"] as const).map((v) => (
            <button
              key={v}
              onClick={() => void choose(v)}
              disabled={!!busy}
              className="group flex flex-col items-center gap-2 rounded-xl border border-slate-800 bg-slate-900/40 px-4 py-6 text-center transition-colors hover:border-brand-500/50 hover:bg-slate-900 disabled:opacity-50"
            >
              <span className="text-base font-bold text-slate-100">
                {v === "msfs2020" ? "MSFS 2020" : "MSFS 2024"}
              </span>
              <span className="text-[11px] leading-relaxed text-slate-400">
                {v === "msfs2020"
                  ? t("simver.opt2020")
                  : t("simver.opt2024")}
              </span>
              {busy === v ? (
                <Loader2 className="mt-1 h-4 w-4 animate-spin text-brand-300" />
              ) : (
                <span className="mt-1 inline-flex items-center gap-1 text-[11px] font-semibold text-brand-300 opacity-0 transition-opacity group-hover:opacity-100">
                  <Check className="h-3 w-3" />
                  {t("simver.choose")}
                </span>
              )}
            </button>
          ))}
        </div>

        <p className="border-t border-slate-800 px-5 py-3 text-[11px] text-slate-500">
          {t("simver.note")}
        </p>
      </motion.div>
    </div>
  );
}
