import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { PlaneLanding } from "lucide-react";
import {
  getCurrentWindow,
  primaryMonitor,
  PhysicalPosition,
} from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { t } from "../lib/i18n";

/**
 * (v6 #2b) OSD de aterrizaje — ventana overlay transparente, always-on-top y
 * click-through (estilo LandingToast). Al tocar pista, el backend emite
 * `landing://osd` con el FPM y el grado; este componente posiciona la ventana
 * (arriba/abajo del monitor primario), muestra el toast y se auto-oculta.
 *
 * Corre en su propia ventana ("osd"); `main.tsx` enruta por label.
 */

interface OsdPayload {
  fpm: number;
  grade: string; // "butter" | "acceptable" | "hard"
  position: number; // 0 = arriba, 1 = abajo
}

const GRADE: Record<string, { color: string; key: string }> = {
  butter: { color: "#3fbf78", key: "osd.butter" },
  acceptable: { color: "#f59e0b", key: "osd.acceptable" },
  hard: { color: "#f43f5e", key: "osd.hard" },
};

export function LandingOsd() {
  const [data, setData] = useState<OsdPayload | null>(null);

  useEffect(() => {
    // Fondo transparente (esta ventana comparte index.html con la app).
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";

    const win = getCurrentWindow();
    win.setIgnoreCursorEvents(true).catch(() => {});

    let hideTimer: number | undefined;
    const unlistenP = listen<OsdPayload>("landing://osd", async (e) => {
      const p = e.payload;
      setData(p);
      try {
        const mon = await primaryMonitor();
        if (mon) {
          const size = await win.outerSize();
          const x =
            mon.position.x + Math.round((mon.size.width - size.width) / 2);
          const y =
            p.position === 1
              ? mon.position.y + mon.size.height - size.height - 90
              : mon.position.y + 70;
          await win.setPosition(new PhysicalPosition(x, y));
        }
      } catch {
        /* ignore */
      }
      await win.show().catch(() => {});
      window.clearTimeout(hideTimer);
      hideTimer = window.setTimeout(() => {
        setData(null);
        win.hide().catch(() => {});
      }, 7000);
    });

    return () => {
      unlistenP.then((u) => u()).catch(() => {});
      window.clearTimeout(hideTimer);
    };
  }, []);

  const g = data ? (GRADE[data.grade] ?? GRADE.acceptable) : null;

  return (
    <div className="flex h-screen w-screen items-center justify-center overflow-hidden bg-transparent p-2">
      <AnimatePresence>
        {data && g && (
          <motion.div
            initial={{ opacity: 0, y: -12, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -8, scale: 0.97 }}
            transition={{ type: "spring", stiffness: 320, damping: 26 }}
            className="flex w-full items-center gap-3 rounded-2xl border border-white/10 bg-slate-950/90 px-4 py-3 shadow-2xl backdrop-blur-md"
            style={{ boxShadow: `0 0 0 1px ${g.color}33, 0 12px 40px #000a` }}
          >
            <div
              className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl"
              style={{ backgroundColor: `${g.color}22`, color: g.color }}
            >
              <PlaneLanding className="h-6 w-6" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-[10px] font-semibold uppercase tracking-[0.18em] text-slate-400">
                {t("osd.title")}
              </div>
              <div className="flex items-baseline gap-1.5">
                <span
                  className="text-3xl font-extrabold leading-none"
                  style={{ color: g.color }}
                >
                  {Math.round(data.fpm)}
                </span>
                <span className="text-sm font-semibold text-slate-400">fpm</span>
              </div>
            </div>
            <div
              className="shrink-0 rounded-lg px-3 py-1.5 text-sm font-bold uppercase tracking-wide"
              style={{ backgroundColor: `${g.color}22`, color: g.color }}
            >
              {t(g.key)}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
