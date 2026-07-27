import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Sparkles, ChevronLeft, ChevronRight, X } from "lucide-react";
import { t } from "../lib/i18n";

/**
 * (v6.2.69) "Novedades" — carrusel que se muestra UNA vez por versión al
 * arrancar, destacando lo nuevo y en especial la integración con flightsim.to.
 * Cada slide trae una imagen (mockup en /public/whatsnew) + título + descripción
 * bilingües (i18n). Navegación libre: dots, ‹ ›, cerrar cuando quieras.
 */
const WHATS_NEW_VERSION = "6.2.68";
const SEEN_KEY = "simfleet.whatsnew.seen.v2";

const SLIDES = [
  { key: "flightsimto", image: "/whatsnew/fsto-browser.svg" },
  { key: "tracking", image: "/whatsnew/fsto-tracking.svg" },
  { key: "gsx", image: "/whatsnew/gsx-offer.svg" },
  { key: "liveries", image: "/whatsnew/liveries.svg" },
] as const;

export function WhatsNewModal() {
  const [open, setOpen] = useState(false);
  const [i, setI] = useState(0);

  useEffect(() => {
    try {
      if (localStorage.getItem(SEEN_KEY) !== WHATS_NEW_VERSION) setOpen(true);
    } catch {
      /* ignore */
    }
  }, []);

  const close = () => {
    try {
      localStorage.setItem(SEEN_KEY, WHATS_NEW_VERSION);
    } catch {
      /* ignore */
    }
    setOpen(false);
  };

  const last = SLIDES.length - 1;
  const slide = SLIDES[i];

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          className="fixed inset-0 z-[70] flex items-center justify-center bg-black/60 p-4"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={close}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 10 }}
            transition={{ duration: 0.16 }}
            onClick={(e) => e.stopPropagation()}
            className="flex max-h-[88vh] w-full max-w-xl flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
          >
            {/* badge + close */}
            <div className="flex items-center justify-between px-5 pt-4">
              <div className="inline-flex items-center gap-1.5 rounded-full bg-sky-500/15 px-2.5 py-1 text-[10px] font-bold uppercase tracking-wider text-sky-300 ring-1 ring-sky-500/30">
                <Sparkles className="h-3 w-3" />
                {t("whatsnew.badge", { version: WHATS_NEW_VERSION })}
              </div>
              <button
                onClick={close}
                title={t("common.dismiss")}
                className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* image + text (scrolls if needed) */}
            <div className="min-h-0 flex-1 overflow-y-auto px-5 pt-3">
              <div className="overflow-hidden rounded-xl border border-slate-800 bg-slate-900">
                <AnimatePresence mode="wait">
                  <motion.img
                    key={slide.key}
                    src={slide.image}
                    alt=""
                    initial={{ opacity: 0, x: 12 }}
                    animate={{ opacity: 1, x: 0 }}
                    exit={{ opacity: 0, x: -12 }}
                    transition={{ duration: 0.18 }}
                    className="aspect-video w-full object-cover"
                  />
                </AnimatePresence>
              </div>
              <AnimatePresence mode="wait">
                <motion.div
                  key={slide.key}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  exit={{ opacity: 0, y: -6 }}
                  transition={{ duration: 0.16 }}
                >
                  <h2 className="mt-4 text-lg font-semibold text-slate-100">
                    {t(`whatsnew.${slide.key}.title`)}
                  </h2>
                  <p className="mt-1 text-sm leading-relaxed text-slate-400">
                    {t(`whatsnew.${slide.key}.desc`)}
                  </p>
                </motion.div>
              </AnimatePresence>
            </div>

            {/* footer: dots + nav */}
            <div className="flex items-center justify-between gap-3 border-t border-slate-800 p-4">
              <button
                onClick={() => setI((n) => Math.max(0, n - 1))}
                disabled={i === 0}
                className="inline-flex h-9 w-9 items-center justify-center rounded-lg border border-slate-800 text-slate-300 transition-colors hover:bg-slate-900 disabled:opacity-30"
              >
                <ChevronLeft className="h-4 w-4" />
              </button>

              <div className="flex items-center gap-1.5">
                {SLIDES.map((s, n) => (
                  <button
                    key={s.key}
                    onClick={() => setI(n)}
                    className={`h-1.5 rounded-full transition-all ${
                      n === i ? "w-5 bg-sky-400" : "w-1.5 bg-slate-700 hover:bg-slate-600"
                    }`}
                  />
                ))}
              </div>

              {i < last ? (
                <button
                  onClick={() => setI((n) => Math.min(last, n + 1))}
                  className="inline-flex h-9 items-center gap-1 rounded-lg bg-slate-800 px-3 text-sm font-medium text-slate-100 transition-colors hover:bg-slate-700"
                >
                  {t("whatsnew.next")}
                  <ChevronRight className="h-4 w-4" />
                </button>
              ) : (
                <button
                  onClick={close}
                  className="inline-flex h-9 items-center rounded-lg bg-sky-500 px-4 text-sm font-semibold text-sky-950 transition-colors hover:bg-sky-400"
                >
                  {t("whatsnew.cta")}
                </button>
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
