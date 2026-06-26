import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { ArrowRight, Check, ImageOff, Sparkles } from "lucide-react";
import { t } from "../lib/i18n";

/**
 * (v6) Modal "Novedades / What's New" — ANTI-SKIP.
 *
 * Reglas (pedido del usuario):
 *  · Instancia principal: se lanza una sola vez cada vez que la app SE
 *    ACTUALIZA (la versión guardada en localStorage difiere de la actual).
 *    En MODO SEGURO se muestra siempre (para revisarlo en pruebas).
 *  · NO se cierra de golpe: sin botón X, el backdrop no cierra, Escape no
 *    cierra. La única salida es avanzar TODAS las slides y pulsar "Entendido".
 *  · Temporizador estricto de 2.0s por slide: el botón "Siguiente" queda
 *    bloqueado (con un anillo de progreso que se llena) hasta que pasen los
 *    2s. Si el usuario lo pulsa antes, se sacude y muestra "Lee el contenido".
 *  · Cada slide DEBE traer una captura/mockup (no solo texto). Si la imagen
 *    falta o no carga, se muestra un placeholder claro.
 *  · Bilingüe: los textos salen por i18n (ES/EN); el modal acepta un array de
 *    objetos { titleKey, bodyKey, image }.
 */

/** localStorage key — guarda la VERSIÓN de la app para la que ya se mostró el
 *  What's New. Así en la instancia principal sólo reaparece cuando la app SE
 *  ACTUALIZA (la versión cambia), una sola vez por versión. */
const SEEN_KEY = "simfleet:whatsnew-version";

/** ms que el usuario debe permanecer en cada slide antes de poder avanzar. */
const SLIDE_LOCK_MS = 2000;

export interface WhatsNewSlide {
  titleKey: string;
  bodyKey: string;
  /** Ruta bajo /public (ej. "/whatsnew/simbrief.svg"). Obligatoria. */
  image: string;
}

/** Slides por defecto — novedades REALES de la v6 (5.5.0). Sustituyen al
 *  changelog viejo (5.1→5.4): cross-link 2020/2024, Hangar con analíticas,
 *  Best Landings (grabación + OSD) y Crew VS. */
const DEFAULT_SLIDES: WhatsNewSlide[] = [
  {
    titleKey: "whatsnew.realdata.title",
    bodyKey: "whatsnew.realdata.body",
    image: "/whatsnew/realdata.svg",
  },
  {
    titleKey: "whatsnew.finance2.title",
    bodyKey: "whatsnew.finance2.body",
    image: "/whatsnew/finance2.svg",
  },
  {
    titleKey: "whatsnew.settings2.title",
    bodyKey: "whatsnew.settings2.body",
    image: "/whatsnew/settings2.svg",
  },
];

/** (v6.2) Slides del What's New. Antes se personalizaba el de Crew VS por
 *  identidad; los slides de esta versión no lo necesitan. */
export function buildWhatsNewSlides(_identity?: string | null): WhatsNewSlide[] {
  return DEFAULT_SLIDES;
}

/** Versión de la app para la que ya se mostró el What's New (o null). */
export function getWhatsNewSeenVersion(): string | null {
  try {
    return localStorage.getItem(SEEN_KEY);
  } catch {
    return null;
  }
}

/** Marca el What's New como visto para `version` (NO abre el modal). Lo usa
 *  App para fijar la línea base en primera instalación y al cerrar el modal. */
export function markWhatsNewSeen(version: string) {
  try {
    localStorage.setItem(SEEN_KEY, version);
  } catch {
    /* ignore */
  }
}

/** ¿Hay que mostrar el What's New en la instancia principal?
 *  `true` siempre que la versión vista guardada NO coincida con la actual —
 *  incluido cuando NO hay línea base (primera vez con esta feature o instalación
 *  fresca). El usuario lo pidió explícito: "debe salir CADA VEZ que se actualiza".
 *  Antes se saltaba la primera vez sin baseline y por eso no salió en 5.4→6. */
export function isUpdateSinceLastSeen(currentVersion: string | null): boolean {
  if (!currentVersion) return false;
  return getWhatsNewSeenVersion() !== currentVersion;
}

export function WhatsNewModal({
  slides = DEFAULT_SLIDES,
  onClose,
}: {
  slides?: WhatsNewSlide[];
  onClose: () => void;
}) {
  const [index, setIndex] = useState(0);
  const [unlocked, setUnlocked] = useState(false);
  const [progress, setProgress] = useState(0); // 0..1 del candado de 2s
  const [nudge, setNudge] = useState(false); // sacudida al pulsar antes de tiempo
  const [imgError, setImgError] = useState(false);

  const slide = slides[index];
  const isLast = index >= slides.length - 1;

  // Candado de 2s por slide — se reinicia en cada cambio de slide.
  useEffect(() => {
    setUnlocked(false);
    setProgress(0);
    setImgError(false);
    const started = Date.now();
    const id = setInterval(() => {
      const p = Math.min(1, (Date.now() - started) / SLIDE_LOCK_MS);
      setProgress(p);
      if (p >= 1) {
        setUnlocked(true);
        clearInterval(id);
      }
    }, 50);
    return () => clearInterval(id);
  }, [index]);

  const advance = () => {
    if (!unlocked) {
      // Pulsó antes de tiempo → sacudir + pista "lee el contenido".
      setNudge(true);
      window.setTimeout(() => setNudge(false), 450);
      return;
    }
    if (isLast) {
      // El persistir la versión vista lo hace App en onClose (en modo seguro
      // NO persiste, para que el What's New reaparezca siempre en pruebas).
      onClose();
    } else {
      setIndex((i) => i + 1);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[200] flex items-center justify-center bg-slate-950/85 p-4 backdrop-blur-sm"
      // (anti-skip) el backdrop NO cierra.
      onClick={(e) => e.stopPropagation()}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 10 }}
        animate={
          nudge
            ? { opacity: 1, scale: 1, y: 0, x: [0, -8, 8, -6, 6, 0] }
            : { opacity: 1, scale: 1, y: 0, x: 0 }
        }
        transition={{ duration: nudge ? 0.4 : 0.2 }}
        className="w-[min(560px,calc(100vw-2rem))] overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
      >
        {/* Imagen / mockup — OBLIGATORIO. Placeholder si falla. */}
        <div className="relative aspect-[16/9] w-full overflow-hidden bg-slate-900">
          {!imgError ? (
            <img
              src={slide.image}
              alt={t(slide.titleKey)}
              className="h-full w-full object-cover"
              onError={() => setImgError(true)}
            />
          ) : (
            <div className="flex h-full w-full flex-col items-center justify-center gap-2 text-slate-600">
              <ImageOff className="h-8 w-8" />
              <span className="text-[11px]">{slide.image}</span>
            </div>
          )}
          <div className="absolute left-3 top-3 inline-flex items-center gap-1.5 rounded-full bg-brand-500/90 px-2.5 py-1 text-[10px] font-bold uppercase tracking-wider text-white shadow">
            <Sparkles className="h-3 w-3" />
            {t("whatsnew.badge")}
          </div>
        </div>

        <div className="p-5">
          <h2 className="text-base font-semibold text-slate-100">
            {t(slide.titleKey)}
          </h2>
          <p className="mt-1.5 text-sm leading-relaxed text-slate-400">
            {t(slide.bodyKey)}
          </p>

          <div className="mt-5 flex items-center justify-between">
            {/* Puntos de progreso */}
            <div className="flex items-center gap-1.5">
              {slides.map((_, i) => (
                <span
                  key={i}
                  className={`h-1.5 rounded-full transition-all ${
                    i === index
                      ? "w-5 bg-brand-400"
                      : i < index
                        ? "w-1.5 bg-slate-500"
                        : "w-1.5 bg-slate-700"
                  }`}
                />
              ))}
            </div>

            <div className="flex items-center gap-2">
              <AnimatePresence>
                {!unlocked && nudge && (
                  <motion.span
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    className="text-[11px] font-medium text-amber-300"
                  >
                    {t("whatsnew.read_first")}
                  </motion.span>
                )}
              </AnimatePresence>
              <button
                onClick={advance}
                aria-disabled={!unlocked}
                className={`relative inline-flex items-center gap-1.5 overflow-hidden rounded-md px-4 py-2 text-xs font-medium transition-colors ${
                  unlocked
                    ? "bg-brand-500 text-white hover:bg-brand-400"
                    : "cursor-not-allowed bg-slate-800 text-slate-400"
                }`}
              >
                {/* Barra de progreso del candado (2s) */}
                {!unlocked && (
                  <span
                    className="absolute inset-y-0 left-0 bg-slate-700"
                    style={{ width: `${progress * 100}%` }}
                  />
                )}
                <span className="relative z-10 inline-flex items-center gap-1.5">
                  {isLast ? (
                    <>
                      {t("whatsnew.done")}
                      <Check className="h-3.5 w-3.5" />
                    </>
                  ) : (
                    <>
                      {t("whatsnew.next")}
                      <ArrowRight className="h-3.5 w-3.5" />
                    </>
                  )}
                </span>
              </button>
            </div>
          </div>
        </div>
      </motion.div>
    </div>
  );
}
