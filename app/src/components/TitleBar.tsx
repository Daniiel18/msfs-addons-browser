import { useEffect, useState } from "react";
import { Minus, Square, X, Copy } from "lucide-react";
import { Plane, Globe2 } from "lucide-react";
import { LiveUpdateBadge } from "./LiveUpdateBadge";
import { t } from "../lib/i18n";

/**
 * Custom titlebar dark — reemplaza al titlebar nativo de Windows.
 *
 * La ventana corre con `decorations: false` en `tauri.conf.json`,
 * por eso pintamos nuestro propio header con:
 *   · `data-tauri-drag-region` para arrastrar la ventana.
 *   · Logo + nombre alineados a la izquierda.
 *   · Botones de minimizar / restaurar-maximizar / cerrar a la derecha.
 *
 * La altura fija es 36px para no robar demasiado espacio al
 * contenido. El color de fondo (`bg-slate-950`) es el mismo del
 * resto del chrome de la app para que se sienta continuo.
 */
export function TitleBar() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        const m = await win.isMaximized();
        if (!cancelled) setMaximized(m);
        // Re-leer cuando el usuario maximiza/restaura por otra vía
        // (doble-click en el drag region, atajos OS).
        const unlisten = await win.onResized(async () => {
          if (cancelled) return;
          const next = await win.isMaximized().catch(() => false);
          setMaximized(next);
        });
        return () => {
          unlisten();
        };
      } catch {
        // No-Tauri (vitest, demo) — silenciamos.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const callWin = async (op: "minimize" | "toggleMaximize" | "close") => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      if (op === "minimize") await win.minimize();
      else if (op === "toggleMaximize") await win.toggleMaximize();
      else if (op === "close") await win.close();
    } catch (e) {
      console.warn("titlebar window op failed:", e);
    }
  };

  return (
    <div
      data-tauri-drag-region
      // (v1.1.2) Sticky al top con z-index alto para que los botones
      // de minimizar/maximizar/cerrar nunca se pierdan al scrollear.
      // Antes la barra scroleaba con el contenido y el usuario se
      // quedaba sin manera de cerrar/maximizar la ventana.
      className="sticky top-0 z-50 flex h-9 select-none items-center justify-between border-b border-slate-800/80 bg-slate-950 px-3 relative"
    >
      {/* Logo + título — atrapados también por el drag-region porque
          son hijos del div padre con data-tauri-drag-region. */}
      <div
        data-tauri-drag-region
        className="pointer-events-none flex items-center gap-2"
      >
        <div className="flex h-5 w-5 items-center justify-center rounded-md bg-emerald-500/20 ring-1 ring-emerald-500/40">
          <Plane className="h-3 w-3 text-emerald-300" />
        </div>
        <span className="text-[11px] font-semibold tracking-wide text-slate-300">
          SimFleet
        </span>
        <Globe2 className="h-3 w-3 text-slate-500" />
      </div>

      {/* (v2.1.0) Badge de update en vivo — entre el título y los
          controles de ventana, centrado en el titlebar. Sólo visible
          cuando hay una versión más nueva disponible. */}
      <div className="pointer-events-none absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2">
        <LiveUpdateBadge />
      </div>

      {/* Window controls */}
      <div className="flex items-center">
        <button
          onClick={() => callWin("minimize")}
          title={t("titlebar.minimize")}
          className="inline-flex h-9 w-10 items-center justify-center text-slate-400 hover:bg-slate-800 hover:text-slate-100"
        >
          <Minus className="h-3.5 w-3.5" />
        </button>
        <button
          onClick={() => callWin("toggleMaximize")}
          title={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
          className="inline-flex h-9 w-10 items-center justify-center text-slate-400 hover:bg-slate-800 hover:text-slate-100"
        >
          {maximized ? (
            <Copy className="h-3 w-3" />
          ) : (
            <Square className="h-3 w-3" />
          )}
        </button>
        <button
          onClick={() => callWin("close")}
          title={t("common.close")}
          className="inline-flex h-9 w-10 items-center justify-center text-slate-400 hover:bg-rose-500/90 hover:text-white"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}
