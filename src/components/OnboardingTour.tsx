import { useEffect, useLayoutEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ChevronLeft, ChevronRight, Sparkles, X } from "lucide-react";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useAppStore } from "../stores/useAppStore";
import { t } from "../lib/i18n";

/**
 * Tour de bienvenida guiado — recorre las 5 pantallas principales de
 * la app + la configuración esencial, **cambiando de pantalla solo**
 * a medida que avanza (el usuario no tiene que navegar a mano).
 *
 * Se dispara automáticamente la primera vez que la app arranca y
 * `pref_onboarding_completed` no está en `true`. El usuario puede
 * saltarlo en cualquier paso, o relanzarlo desde Settings → Tour.
 *
 * Implementación:
 *   · Cada step puede declarar:
 *       - `setView`      → la vista principal que la app debe mostrar
 *                          (el tour la activa vía el store antes de medir).
 *       - `openSettings` → abre/cierra el modal de Configuración.
 *       - `target`       → `data-tour-id="..."` a destacar con spotlight.
 *     Si un step no tiene `target`, el tooltip va centrado (pasos de
 *     bienvenida / cierre).
 *   · Tras cambiar de vista/abrir modal, el DOM tarda un tick (+ la
 *     transición de framer-motion). Por eso buscamos el target con
 *     reintentos (120/300/600/1000ms) hasta encontrarlo.
 *   · Spotlight = un div transparente sobre el target con un
 *     `box-shadow` gigante que oscurece todo lo demás. Sin clip-path.
 *
 * Sin librería externa — `framer-motion` + posicionamiento relativo
 * al viewport bastan. Bundle limpio.
 */

type TourView = "dashboard" | "search" | "map" | "addons" | "flightbook";

interface TourStep {
  /** Vista principal a mostrar antes de medir el target. */
  setView?: TourView;
  /** Abre (true) / cierra (false/undefined) el modal de Configuración. */
  openSettings?: boolean;
  /** Selector del elemento a destacar (`[data-tour-id="X"]`). Opcional:
   *  sin él, el tooltip va centrado. */
  target?: string;
  title: string;
  body: string;
  placement?: "bottom" | "top" | "left" | "right";
}

/** Returns the localized tour steps. Re-derive on language change. */
function buildSteps(): TourStep[] {
  return [
    // 0 — Bienvenida (centrado).
    {
      title: t("tour.welcome.title"),
      body: t("tour.welcome.body"),
    },
    // 1..5 — Las 5 pantallas principales: el tour navega a cada una.
    {
      setView: "dashboard",
      target: "[data-tour-id='nav-dashboard']",
      title: t("tour.dashboard.title"),
      body: t("tour.dashboard.body"),
      placement: "bottom",
    },
    {
      setView: "search",
      target: "[data-tour-id='nav-search']",
      title: t("tour.search.title"),
      body: t("tour.search.body"),
      placement: "bottom",
    },
    {
      setView: "map",
      target: "[data-tour-id='nav-map']",
      title: t("tour.map.title"),
      body: t("tour.map.body"),
      placement: "bottom",
    },
    {
      setView: "addons",
      target: "[data-tour-id='nav-addons']",
      title: t("tour.addons.title"),
      body: t("tour.addons.body"),
      placement: "bottom",
    },
    {
      setView: "flightbook",
      target: "[data-tour-id='nav-flightbook']",
      title: t("tour.flightbook.title"),
      body: t("tour.flightbook.body"),
      placement: "bottom",
    },
    // 6 — Notificaciones (chrome global).
    {
      target: "[data-tour-id='header-notifications']",
      title: t("tour.notifications.title"),
      body: t("tour.notifications.body"),
      placement: "bottom",
    },
    // 7 — La rueda de Configuración (aún sin abrir el modal).
    {
      openSettings: false,
      target: "[data-tour-id='header-settings']",
      title: t("tour.settings.title"),
      body: t("tour.settings.body"),
      placement: "bottom",
    },
    // 8..10 — Dentro de Configuración: secciones esenciales.
    {
      openSettings: true,
      target: "[data-tour-id='settings-general']",
      title: t("tour.settings_units.title"),
      body: t("tour.settings_units.body"),
      placement: "right",
    },
    {
      openSettings: true,
      target: "[data-tour-id='settings-folders']",
      title: t("tour.settings_folders.title"),
      body: t("tour.settings_folders.body"),
      placement: "right",
    },
    {
      openSettings: true,
      target: "[data-tour-id='settings-cloud']",
      title: t("tour.settings_cloud.title"),
      body: t("tour.settings_cloud.body"),
      placement: "right",
    },
    // 11 — Cierre (centrado, cerramos el modal).
    {
      openSettings: false,
      title: t("tour.finish.title"),
      body: t("tour.finish.body"),
    },
  ];
}

interface Props {
  onClose: () => void;
  /** Controla la apertura del modal de Configuración desde el tour. */
  onSettingsOpenChange: (open: boolean) => void;
}

export function OnboardingTour({ onClose, onSettingsOpenChange }: Props) {
  const [stepIdx, setStepIdx] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const setOnboardingCompleted = useSettingsStore((s) => s.setOnboardingCompleted);

  // Resolve translated steps once per mount.
  const STEPS = useMemo(() => buildSteps(), []);
  const step = STEPS[stepIdx];

  // Aplica los efectos de navegación del step (cambiar vista / abrir
  // modal) y mide el target con reintentos para tolerar transiciones.
  useLayoutEffect(() => {
    if (step.setView) {
      useAppStore.getState().setView(step.setView);
    }
    onSettingsOpenChange(!!step.openSettings);

    let cancelled = false;
    const timers: number[] = [];
    const find = () => {
      if (cancelled) return;
      if (!step.target) {
        setRect(null);
        return;
      }
      const el = document.querySelector(step.target) as HTMLElement | null;
      if (!el) {
        setRect(null);
        return;
      }
      el.scrollIntoView({ behavior: "smooth", block: "center" });
      setRect(el.getBoundingClientRect());
    };
    find();
    // El cambio de vista + la transición del modal (~180ms) hacen que
    // el target no exista en el primer paint. Reintentamos.
    [120, 300, 600, 1000].forEach((d) =>
      timers.push(window.setTimeout(find, d)),
    );
    const onResize = () => find();
    window.addEventListener("resize", onResize);
    return () => {
      cancelled = true;
      timers.forEach((id) => clearTimeout(id));
      window.removeEventListener("resize", onResize);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stepIdx]);

  // Atajos: ← → para navegar, Esc para cerrar.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") finish();
      else if (e.key === "ArrowRight") next();
      else if (e.key === "ArrowLeft") prev();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stepIdx]);

  const next = () => {
    if (stepIdx >= STEPS.length - 1) {
      finish();
    } else {
      setStepIdx((i) => i + 1);
    }
  };
  const prev = () => setStepIdx((i) => Math.max(0, i - 1));

  const finish = () => {
    // Cerramos el modal de settings si quedó abierto y persistimos que
    // el usuario ya pasó el tour (saltado o completado).
    onSettingsOpenChange(false);
    void setOnboardingCompleted(true).catch(() => {});
    if (typeof window !== "undefined") {
      window.sessionStorage.setItem(
        "msfs-addons:onboarding-skip-session",
        "1",
      );
    }
    onClose();
  };

  const tooltipPos = computeTooltipPosition(rect, step.placement ?? "bottom");
  const isLast = stepIdx === STEPS.length - 1;

  return (
    <AnimatePresence>
      <motion.div
        key="tour-root"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.18 }}
        className="fixed inset-0 z-[100] pointer-events-none"
      >
        {/* Backdrop con spotlight — el box-shadow del highlighter
            oscurece todo menos el rect del target. Si no hay target
            (bienvenida/cierre), oscurecemos toda la pantalla. */}
        {rect ? (
          <div
            className="pointer-events-auto absolute"
            style={{
              top: rect.top - 6,
              left: rect.left - 6,
              width: rect.width + 12,
              height: rect.height + 12,
              boxShadow: "0 0 0 9999px rgba(2, 6, 23, 0.78)",
              borderRadius: 12,
              border: "2px solid rgb(34, 197, 94)",
              transition: "all 0.25s cubic-bezier(0.4, 0, 0.2, 1)",
            }}
            onClick={(e) => e.stopPropagation()}
          />
        ) : (
          <div className="pointer-events-auto absolute inset-0 bg-slate-950/80" />
        )}

        {/* Tooltip */}
        <motion.div
          key={`step-${stepIdx}`}
          initial={{ opacity: 0, y: 6 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 6 }}
          transition={{ duration: 0.18 }}
          style={{
            top: tooltipPos.top,
            left: tooltipPos.left,
            maxWidth: 360,
          }}
          className="pointer-events-auto absolute rounded-2xl border border-emerald-500/40 bg-slate-950/95 p-4 shadow-2xl backdrop-blur"
        >
          <div className="flex items-start gap-2">
            <div className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-lg bg-emerald-500/15 ring-1 ring-emerald-500/40">
              <Sparkles className="h-3.5 w-3.5 text-emerald-300" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2">
                <span className="text-[10px] font-medium uppercase tracking-wider text-emerald-400">
                  {t("tour.step_of", {
                    current: String(stepIdx + 1),
                    total: String(STEPS.length),
                  })}
                </span>
              </div>
              <h3 className="mt-1 text-sm font-semibold text-slate-100">
                {step.title}
              </h3>
              <p className="mt-1 text-[12px] leading-relaxed text-slate-300">
                {step.body}
              </p>
            </div>
            <button
              onClick={finish}
              title={t("tour.skip")}
              className="shrink-0 rounded-md p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-100"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>

          {/* Progress bar */}
          <div className="mt-3 flex gap-1">
            {STEPS.map((_, i) => (
              <div
                key={i}
                className={`h-1 flex-1 rounded-full transition-colors ${
                  i <= stepIdx ? "bg-emerald-400" : "bg-slate-800"
                }`}
              />
            ))}
          </div>

          <div className="mt-3 flex items-center justify-between gap-2">
            <button
              onClick={finish}
              className="text-[11px] text-slate-400 hover:text-slate-200"
            >
              {t("tour.skip")}
            </button>
            <div className="flex items-center gap-1.5">
              <button
                onClick={prev}
                disabled={stepIdx === 0}
                className="inline-flex items-center gap-1 rounded-md border border-slate-800 px-2.5 py-1 text-[11px] text-slate-300 hover:border-slate-700 disabled:opacity-30"
              >
                <ChevronLeft className="h-3 w-3" />
                {t("common.back")}
              </button>
              <button
                onClick={next}
                className="inline-flex items-center gap-1 rounded-md bg-emerald-500 px-3 py-1 text-[11px] font-medium text-emerald-950 hover:bg-emerald-400"
              >
                {isLast ? t("tour.finish") : t("tour.next")}
                {!isLast && <ChevronRight className="h-3 w-3" />}
              </button>
            </div>
          </div>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}

/**
 * Calcula la posición del tooltip relativa al rect del target.
 * Por defecto va debajo; se ajusta automáticamente si el espacio
 * inferior es insuficiente. Mantiene el tooltip dentro del viewport.
 */
function computeTooltipPosition(
  rect: DOMRect | null,
  placement: "bottom" | "top" | "left" | "right",
): { top: number; left: number } {
  // Sin target → centro de pantalla.
  if (!rect) {
    return {
      top: window.innerHeight / 2 - 100,
      left: window.innerWidth / 2 - 180,
    };
  }
  const TOOLTIP_W = 360;
  const TOOLTIP_H = 220; // estimado
  const GAP = 14;

  let top: number;
  let left: number;

  if (placement === "right") {
    // A la derecha del target (usado dentro del modal de settings, que
    // está centrado — hay espacio a la derecha). Fallback a izquierda
    // si no cabe.
    top = rect.top;
    left = rect.right + GAP;
    if (left + TOOLTIP_W > window.innerWidth - 12) {
      left = rect.left - TOOLTIP_W - GAP;
    }
  } else if (placement === "left") {
    top = rect.top;
    left = rect.left - TOOLTIP_W - GAP;
    if (left < 12) left = rect.right + GAP;
  } else {
    // bottom / top
    top = rect.bottom + GAP;
    left = rect.left + rect.width / 2 - TOOLTIP_W / 2;
    if (placement === "top" || top + TOOLTIP_H > window.innerHeight - 12) {
      top = rect.top - TOOLTIP_H - GAP;
      if (top < 12) top = 12;
    }
  }

  // Clamp al viewport.
  if (top < 12) top = 12;
  if (top + TOOLTIP_H > window.innerHeight - 12) {
    top = Math.max(12, window.innerHeight - TOOLTIP_H - 12);
  }
  if (left < 12) left = 12;
  if (left + TOOLTIP_W > window.innerWidth - 12) {
    left = window.innerWidth - TOOLTIP_W - 12;
  }
  return { top, left };
}
