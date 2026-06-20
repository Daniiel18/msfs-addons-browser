import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { PlaneLanding, ArrowUp } from "lucide-react";
import {
  getCurrentWindow,
  primaryMonitor,
  PhysicalPosition,
} from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { t, preloadLocale } from "../lib/i18n";
import { api } from "../lib/tauri";

/**
 * (v6 #2b) OSD de aterrizaje — overlay transparente, always-on-top y
 * click-through (estilo LandingToast). El watcher emite `landing://osd` en el
 * MISMO instante del touchdown con FPM, G, viento, pitch y roll (todos
 * simvars que ya leía). Esta ventana posiciona, muestra el toast y se auto-
 * oculta. Corre en su propia ventana ("osd"); `main.tsx` enruta por label.
 */

interface OsdPayload {
  fpm: number;
  grade: string; // "butter" | "acceptable" | "hard"
  gForce?: number;
  pitch?: number;
  roll?: number;
  windDir?: number;
  windKt?: number;
  headingDeg?: number;
  bounced?: boolean;
  position?: number; // legacy (prueba) — normalmente viene de la config
}

const GRADE: Record<string, { color: string; key: string }> = {
  butter: { color: "#3fbf78", key: "osd.butter" },
  acceptable: { color: "#f59e0b", key: "osd.acceptable" },
  hard: { color: "#f43f5e", key: "osd.hard" },
};

export function LandingOsd() {
  const [data, setData] = useState<OsdPayload | null>(null);

  useEffect(() => {
    document.documentElement.style.background = "transparent";
    document.body.style.background = "transparent";

    const win = getCurrentWindow();
    api.osdDebug(`OSD montado (label=${win.label})`).catch(() => {});
    win.setIgnoreCursorEvents(true).catch(() => {});

    let hideTimer: number | undefined;
    const unlistenP = listen<OsdPayload>("landing://osd", async (e) => {
      const p = e.payload;
      api.osdDebug(`evento recibido fpm=${p.fpm} grade=${p.grade}`).catch(() => {});

      // (fix idioma) Re-leemos el locale en cada evento — la ventana OSD no
      // se recarga al cambiar idioma; así el toast sale siempre en el actual.
      try {
        preloadLocale();
      } catch {
        /* ignore */
      }

      // Config actual: si el OSD está desactivado no mostramos; posición.
      let position = p.position ?? 0;
      try {
        const cfg = await api.recordingConfig();
        if (!cfg.osdEnabled) {
          api.osdDebug("osd desactivado en ajustes; no se muestra").catch(() => {});
          return;
        }
        position = cfg.osdPosition;
      } catch {
        /* usar default */
      }

      setData(p);

      try {
        const mon = await primaryMonitor();
        if (mon) {
          const size = await win.outerSize();
          const x =
            mon.position.x + Math.round((mon.size.width - size.width) / 2);
          const y =
            position === 1
              ? mon.position.y + mon.size.height - size.height - 90
              : mon.position.y + 70;
          await win.setPosition(new PhysicalPosition(x, y));
        }
      } catch (err) {
        api.osdDebug(`error posicionando: ${String(err)}`).catch(() => {});
      }

      // Reforzar always-on-top para aparecer sobre MSFS (en ventana/borderless).
      await win.setAlwaysOnTop(true).catch(() => {});
      await win.show().catch((err) =>
        api.osdDebug(`error show: ${String(err)}`).catch(() => {}),
      );
      api.osdDebug("show() llamado").catch(() => {});

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
  const windRel =
    data && data.windDir != null ? data.windDir - (data.headingDeg ?? 0) : 0;

  return (
    <div className="flex h-screen w-screen items-center justify-center overflow-hidden bg-transparent p-1.5">
      <AnimatePresence>
        {data && g && (
          <motion.div
            initial={{ opacity: 0, y: -12, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -8, scale: 0.97 }}
            transition={{ type: "spring", stiffness: 320, damping: 26 }}
            className="flex w-full flex-col items-center rounded-2xl border px-5 py-3.5 text-center"
            style={{
              // Panel OPACO (sólido) para que se lea sobre cualquier fondo.
              background: "#0b1119",
              borderColor: `${g.color}66`,
              boxShadow: `0 0 26px ${g.color}33, 0 16px 48px #000e`,
            }}
          >
            {/* Cabecera: LANDING + grado (centrado) */}
            <div className="flex items-center justify-center gap-2">
              <PlaneLanding className="h-4 w-4" style={{ color: g.color }} />
              <span className="text-xs font-semibold uppercase tracking-[0.22em] text-slate-300">
                {t("osd.title")}
              </span>
              <span
                className="rounded-md px-2 py-0.5 text-xs font-bold uppercase tracking-wide"
                style={{ backgroundColor: `${g.color}26`, color: g.color }}
              >
                {t(g.key)}
              </span>
              {data.bounced && (
                <span className="rounded-md bg-amber-500/25 px-2 py-0.5 text-xs font-bold uppercase tracking-wide text-amber-300">
                  {t("osd.bounce")}
                </span>
              )}
            </div>

            {/* FPM grande + G */}
            <div className="mt-1.5 flex items-baseline justify-center gap-2">
              <span
                className="text-5xl font-extrabold leading-none"
                style={{ color: g.color }}
              >
                {Math.round(data.fpm)}
              </span>
              <span className="text-xl font-bold text-slate-300">fpm</span>
              {data.gForce != null && (
                <span className="text-xl font-semibold text-slate-400">
                  ({data.gForce.toFixed(2)} G)
                </span>
              )}
            </div>

            {/* Viento (flecha relativa al rumbo + nudos) */}
            {data.windKt != null && (
              <div className="mt-2 flex items-center justify-center gap-1.5 text-lg font-semibold text-sky-300">
                <ArrowUp
                  className="h-5 w-5"
                  style={{ transform: `rotate(${windRel}deg)` }}
                />
                {Math.round(data.windKt)} kt
              </div>
            )}

            {/* Pitch / Roll */}
            {(data.pitch != null || data.roll != null) && (
              <div className="mt-1 text-sm font-medium text-slate-400">
                {t("osd.pitch")}: {Math.round(data.pitch ?? 0)}° &nbsp;·&nbsp;{" "}
                {t("osd.roll")}: {Math.round(data.roll ?? 0)}°
              </div>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
