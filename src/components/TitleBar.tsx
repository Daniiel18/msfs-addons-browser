import { useEffect, useState, type ReactNode } from "react";
import { Minus, Square, X, Copy } from "lucide-react";
import { isTauri } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * Barra superior única de la app (Rediseño F1).
 *
 * Antes había DOS filas: el titlebar custom (36px, logo + controles de
 * ventana) y la cabecera de la app (logo repetido + accesos). Ahora es
 * una sola fila de 44px a la derecha del menú lateral:
 *   · `leading`  → buscador global (abre la paleta Ctrl+K).
 *   · `actions`  → accesos globales (notificaciones, descargas, piloto…).
 *   · Controles de ventana (minimizar / maximizar / cerrar) a la derecha.
 *
 * La ventana corre con `decorations: false` en `tauri.conf.json`, así que
 * esta barra conserva `data-tauri-drag-region`: el fondo y el espaciador
 * arrastran la ventana (y doble clic maximiza); los botones no.
 */
export function TitleBar({
  leading,
  actions,
}: {
  leading?: ReactNode;
  actions?: ReactNode;
}) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri) return;
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const win = getCurrentWindow();
        const m = await win.isMaximized();
        if (!cancelled) setMaximized(m);
        // Re-leer cuando el usuario maximiza/restaura por otra vía
        // (doble-click en el drag region, atajos OS).
        const off = await win.onResized(async () => {
          if (cancelled) return;
          const next = await win.isMaximized().catch(() => false);
          setMaximized(next);
        });
        if (cancelled) off();
        else unlisten = off;
      } catch {
        // No-Tauri (vitest, demo) — silenciamos.
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
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
    <header
      data-tauri-drag-region
      className="app-shell-header relative z-40 flex h-11 shrink-0 select-none items-center gap-3 border-b border-sf-border/70 bg-sf-bg pl-4"
    >
      {leading}

      {/* Espaciador: zona libre de arrastre entre el buscador y los accesos. */}
      <div data-tauri-drag-region className="h-full min-w-4 flex-1" />

      {actions && (
        <div className="flex shrink-0 items-center gap-2">{actions}</div>
      )}

      {isTauri ? (
        <div className="ml-1 flex h-full shrink-0 items-center border-l border-sf-border/70">
          <button
            onClick={() => callWin("minimize")}
            title={t("titlebar.minimize")}
            className="inline-flex h-full w-11 items-center justify-center text-sf-muted hover:bg-sf-raised hover:text-sf-text"
          >
            <Minus className="h-3.5 w-3.5" />
          </button>
          <button
            onClick={() => callWin("toggleMaximize")}
            title={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
            className="inline-flex h-full w-11 items-center justify-center text-sf-muted hover:bg-sf-raised hover:text-sf-text"
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
            className="inline-flex h-full w-11 items-center justify-center text-sf-muted hover:bg-rose-500/90 hover:text-white"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      ) : (
        <div className="w-2 shrink-0" />
      )}
    </header>
  );
}
