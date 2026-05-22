import { useEffect, useLayoutEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ChevronLeft, ChevronRight, Sparkles, X } from "lucide-react";
import { useSettingsStore } from "../stores/useSettingsStore";

/**
 * Tour de bienvenida — 6 pasos que muestran las áreas principales
 * de la app a un usuario nuevo. Se dispara automáticamente la
 * primera vez que la app arranca y `pref_onboarding_completed`
 * no está en `true`. El usuario puede saltarlo en cualquier paso
 * con el botón "Saltar tour".
 *
 * Implementación:
 *   · Cada step apunta a un `data-tour-id="..."` colocado en el
 *     elemento real de la UI (tabs, header buttons, etc.).
 *   · Calculamos el bounding box del target y dibujamos un
 *     "spotlight" (recorte transparente sobre un backdrop oscuro).
 *   · Tooltip con título + descripción + Skip/Anterior/Siguiente.
 *   · Si el target no existe en este viewport (vista no montada),
 *     saltamos el paso o cambiamos vista forzadamente.
 *
 * Sin librería externa — `framer-motion` y posicionamiento
 * relativo al viewport bastan para 6 pasos. Bundle limpio.
 */

interface TourStep {
  /** Selector del elemento a destacar (`[data-tour-id="X"]`). */
  target: string;
  /** Vista que la UI debe mostrar para que el target esté en el
   *  DOM. Si está en el chrome global (header) se omite. */
  requiresView?: "dashboard" | "search" | "map" | "addons";
  title: string;
  body: string;
  placement?: "bottom" | "top" | "left" | "right";
}

const STEPS: TourStep[] = [
  {
    target: "[data-tour-id='nav-dashboard']",
    title: "Dashboard",
    body: "Tu panel de inicio: total de paquetes instalados, espacio en disco, top desarrolladores y los más grandes. Se actualiza automáticamente cuando cambia algo en tu carpeta Community.",
    placement: "bottom",
  },
  {
    target: "[data-tour-id='nav-search']",
    title: "Buscar en catálogos",
    body: "Busca aeropuertos, aviones, liveries y mods en SceneryAddons y Simplaza. Cada card muestra si ya lo tienes instalado y si hay update disponible.",
    placement: "bottom",
  },
  {
    target: "[data-tour-id='nav-map']",
    title: "Mapa mundial",
    body: "Vista mundial con todos tus aeropuertos instalados como puntos verdes. Click en uno abre su detalle. El badge ámbar marca los que tienen update disponible.",
    placement: "bottom",
  },
  {
    target: "[data-tour-id='nav-addons']",
    title: "Tus addons",
    body: "Aviones, liveries, instrumentos y mods que tienes instalados. Filtra por tipo, busca por nombre, y abre cada uno para reinstalar o desinstalar.",
    placement: "bottom",
  },
  {
    target: "[data-tour-id='nav-flightbook']",
    title: "FlightBook (vuelos reales)",
    body: "Bitácora de tus vuelos en MSFS, capturada automáticamente con SimConnect. ICAOs, gates, FPM al aterrizar, fuel, pasajeros (vía SimBrief OFP). Click en un vuelo para ver su ruta real en el globo. Cuando estás volando, el avión naranja te marca dónde vas.",
    placement: "bottom",
  },
  {
    target: "[data-tour-id='header-settings']",
    title: "Configuración",
    body: "SimBrief Pilot ID, autostart con Windows, tema dark/light, backup de Community, exportar/importar inventario, perfiles GSX… Todo está aquí.",
    placement: "bottom",
  },
  {
    target: "[data-tour-id='header-notifications']",
    title: "Notificaciones",
    body: "Aquí aparecen las actualizaciones disponibles para tus paquetes y para la app. La app comprueba al arrancar y al ganar foco — sin botones manuales.",
    placement: "bottom",
  },
];

interface Props {
  onClose: () => void;
}

export function OnboardingTour({ onClose }: Props) {
  const [stepIdx, setStepIdx] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const setOnboardingCompleted = useSettingsStore((s) => s.setOnboardingCompleted);

  const step = STEPS[stepIdx];

  // Recalcula la posición del target cuando el step cambia o la
  // ventana cambia de tamaño. `useLayoutEffect` para evitar el
  // flash del tooltip en posición vieja antes de re-render.
  useLayoutEffect(() => {
    const find = () => {
      const el = document.querySelector(step.target) as HTMLElement | null;
      if (!el) {
        setRect(null);
        return;
      }
      setRect(el.getBoundingClientRect());
      // Scroll al elemento si está fuera del viewport.
      el.scrollIntoView({ behavior: "smooth", block: "nearest" });
    };
    find();
    // Pequeño debounce para que el tab/sidebar termine su transición
    // antes de que mediamos.
    const t = setTimeout(find, 200);
    const onResize = () => find();
    window.addEventListener("resize", onResize);
    return () => {
      clearTimeout(t);
      window.removeEventListener("resize", onResize);
    };
  }, [step.target, stepIdx]);

  // Atajos: ← → para navegar, Esc para cerrar.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") finish(false);
      else if (e.key === "ArrowRight") next();
      else if (e.key === "ArrowLeft") prev();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stepIdx]);

  const next = () => {
    if (stepIdx >= STEPS.length - 1) {
      finish(true);
    } else {
      setStepIdx((i) => i + 1);
    }
  };
  const prev = () => setStepIdx((i) => Math.max(0, i - 1));

  const finish = (_completed: boolean) => {
    // Persistimos en backend siempre que el usuario "pasó" —
    // saltado o completado. Tampoco le insistimos en esta sesión.
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
        {/* Backdrop con spotlight — usamos box-shadow del propio
            highlighter para oscurecer todo menos el rect del target.
            Un sólo div, sin clip-path complicado. Si el target no
            se encontró, oscurecemos toda la pantalla. */}
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
                  Paso {stepIdx + 1} de {STEPS.length}
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
              onClick={() => finish(false)}
              title="Saltar tour"
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
              onClick={() => finish(false)}
              className="text-[11px] text-slate-400 hover:text-slate-200"
            >
              Saltar tour
            </button>
            <div className="flex items-center gap-1.5">
              <button
                onClick={prev}
                disabled={stepIdx === 0}
                className="inline-flex items-center gap-1 rounded-md border border-slate-800 px-2.5 py-1 text-[11px] text-slate-300 hover:border-slate-700 disabled:opacity-30"
              >
                <ChevronLeft className="h-3 w-3" />
                Atrás
              </button>
              <button
                onClick={next}
                className="inline-flex items-center gap-1 rounded-md bg-emerald-500 px-3 py-1 text-[11px] font-medium text-emerald-950 hover:bg-emerald-400"
              >
                {stepIdx === STEPS.length - 1 ? "Empezar" : "Siguiente"}
                {stepIdx < STEPS.length - 1 && <ChevronRight className="h-3 w-3" />}
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
  const TOOLTIP_H = 200; // estimado
  const GAP = 14;

  let top = rect.bottom + GAP;
  let left = rect.left + rect.width / 2 - TOOLTIP_W / 2;

  // Si placement=top o no cabe debajo, lo ponemos arriba.
  if (placement === "top" || top + TOOLTIP_H > window.innerHeight - 12) {
    top = rect.top - TOOLTIP_H - GAP;
    if (top < 12) top = 12;
  }

  // Clamp horizontal al viewport.
  if (left < 12) left = 12;
  if (left + TOOLTIP_W > window.innerWidth - 12) {
    left = window.innerWidth - TOOLTIP_W - 12;
  }
  return { top, left };
}
