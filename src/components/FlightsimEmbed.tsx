import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Globe2,
  Home,
  Loader2,
  RotateCw,
} from "lucide-react";
import { api, isTauri } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * (19/10) flightsim.to DENTRO de Explore.
 *
 * Este componente sólo reserva el hueco: la web la pinta un webview nativo
 * hijo de la ventana principal (`commands/flightsim_embed.rs`) que colocamos
 * exactamente encima. Como un webview nativo queda SIEMPRE por encima del
 * HTML de SimFleet, lo ocultamos mientras haya abierto un modal (los modales
 * usan `fixed inset-0`) o un panel marcado con `data-sf-overlay`
 * (notificaciones, descargas, menú lateral superpuesto).
 */
const OVERLAY_SELECTOR = ".fixed.inset-0, [data-sf-overlay]";
const HOME = "https://flightsim.to/";

export function FlightsimEmbed() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [ready, setReady] = useState(false);
  const [url, setUrl] = useState(HOME);
  // Se incrementa cuando el webview se cierra (botón "Cancelar" del overlay
  // de descarga) para volver a crearlo.
  const [epoch, setEpoch] = useState(0);

  useEffect(() => {
    if (!isTauri) return;
    const offUrl = listen<string>("fsto-embed://url", (e) => setUrl(e.payload));
    const offClosed = listen("fsto-embed://closed", () => {
      setReady(false);
      setEpoch((n) => n + 1);
    });
    return () => {
      void offUrl.then((f) => f());
      void offClosed.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const el = hostRef.current;
    if (!isTauri || !el) return;
    let alive = true;
    let visible = false;
    let last = "";
    let raf = 0;
    const sync = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        if (!alive) return;
        const r = el.getBoundingClientRect();
        const covered = !!document.querySelector(OVERLAY_SELECTOR);
        if (covered || r.width < 40 || r.height < 40) {
          if (visible) {
            visible = false;
            void api.fstoEmbedHide();
          }
          return;
        }
        const b = {
          x: Math.round(r.left),
          y: Math.round(r.top),
          width: Math.round(r.width),
          height: Math.round(r.height),
        };
        const key = `${b.x},${b.y},${b.width},${b.height}`;
        if (!visible) {
          visible = true;
          last = key;
          api
            .fstoEmbedShow(b)
            .then(() => alive && setReady(true))
            .catch(() => {
              visible = false;
            });
        } else if (key !== last) {
          last = key;
          void api.fstoEmbedBounds(b);
        }
      });
    };
    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(el);
    window.addEventListener("resize", sync);
    const mo = new MutationObserver(sync);
    mo.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "data-sf-overlay"],
    });
    return () => {
      alive = false;
      cancelAnimationFrame(raf);
      ro.disconnect();
      mo.disconnect();
      window.removeEventListener("resize", sync);
      void api.fstoEmbedHide();
    };
  }, [epoch]);

  const nav = (action: string) => void api.fstoEmbedNav(action);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-surface">
      <div className="flex h-10 shrink-0 items-center gap-1 border-b border-sf-border px-2">
        <ToolbarButton label={t("fsto.back")} onClick={() => nav("back")}>
          <ArrowLeft className="h-4 w-4" />
        </ToolbarButton>
        <ToolbarButton label={t("fsto.forward")} onClick={() => nav("forward")}>
          <ArrowRight className="h-4 w-4" />
        </ToolbarButton>
        <ToolbarButton label={t("fsto.reload")} onClick={() => nav("reload")}>
          <RotateCw className="h-4 w-4" />
        </ToolbarButton>
        <ToolbarButton label={t("fsto.home")} onClick={() => nav("home")}>
          <Home className="h-4 w-4" />
        </ToolbarButton>
        <div className="ml-1 flex min-w-0 flex-1 items-center gap-1.5 rounded-md bg-sf-bg/60 px-2.5 py-1 text-xs text-sf-muted">
          <Globe2 className="h-3.5 w-3.5 shrink-0 text-sky-400" />
          <span className="truncate font-mono">{url}</span>
        </div>
        <ToolbarButton
          label={t("fsto.open_window")}
          onClick={() => void api.openLiveryBrowser(url)}
        >
          <ExternalLink className="h-4 w-4" />
        </ToolbarButton>
      </div>
      <div ref={hostRef} className="relative min-h-0 flex-1">
        {isTauri ? (
          !ready && (
            <div className="absolute inset-0 flex items-center justify-center gap-2 text-sm text-sf-muted">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("fsto.loading")}
            </div>
          )
        ) : (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 px-6 text-center">
            <Globe2 className="h-8 w-8 text-sky-400" />
            <p className="max-w-sm text-sm text-sf-muted">{t("fsto.demo")}</p>
            <a
              href={HOME}
              target="_blank"
              rel="noreferrer"
              className="rounded-lg border border-sf-border bg-sf-raised px-3 py-1.5 text-xs font-medium text-sf-text hover:border-brand-500/40"
            >
              flightsim.to
            </a>
          </div>
        )}
      </div>
    </div>
  );
}

function ToolbarButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={label}
      aria-label={label}
      className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-sf-muted transition-colors hover:bg-sf-raised hover:text-sf-text"
    >
      {children}
    </button>
  );
}
