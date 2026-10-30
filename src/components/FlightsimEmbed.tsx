import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { create } from "zustand";
import { Globe2, Loader2, RotateCw } from "lucide-react";
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

/** (v8.3) ¿El webview de flightsim.to está a la vista? Mientras lo está, tapa
 *  cualquier HTML que caiga encima (p.ej. los tooltips del menú lateral, que
 *  entonces usan el tooltip nativo de Windows). */
export const useFstoEmbedStore = create<{
  visible: boolean;
  setVisible: (v: boolean) => void;
  /** (25/10) Página a la que ir la próxima vez que se muestre (update de
   *  flightsim.to abierta desde el modal o la campanita). */
  pendingUrl: string | null;
  openUrl: (url: string) => void;
}>((set) => ({
  visible: false,
  setVisible: (visible) => set({ visible }),
  pendingUrl: null,
  openUrl: (pendingUrl) => set({ pendingUrl }),
}));

/** La página pendiente, una sola vez. */
function takePendingUrl(): string | undefined {
  const url = useFstoEmbedStore.getState().pendingUrl;
  if (url) useFstoEmbedStore.setState({ pendingUrl: null });
  return url ?? undefined;
}

export function FlightsimEmbed() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const [ready, setReady] = useState(false);
  // (25/10) Foto de la página mientras un panel/modal tapa el hueco (el
  // webview nativo se oculta, si no quedaría por encima): no "desaparece".
  const [frozen, setFrozen] = useState<string | null>(null);
  // Se incrementa cuando el webview se cierra (botón "Cancelar" del overlay
  // de descarga) para volver a crearlo.
  const [epoch, setEpoch] = useState(0);

  useEffect(() => {
    if (!isTauri) return;
    const offClosed = listen("fsto-embed://closed", () => {
      setReady(false);
      setEpoch((n) => n + 1);
    });
    return () => {
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
    // Invalida las fotos que lleguen tarde (el hueco ya volvió a verse).
    let freezeSeq = 0;
    const sync = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        if (!alive) return;
        const r = el.getBoundingClientRect();
        const covered = !!document.querySelector(OVERLAY_SELECTOR);
        if (covered || r.width < 40 || r.height < 40) {
          if (visible) {
            visible = false;
            useFstoEmbedStore.getState().setVisible(false);
            // (25/10) Con un panel/modal encima: foto primero, luego se
            // oculta (como mucho 0,4 s después aunque la foto tarde).
            const seq = ++freezeSeq;
            let hidden = false;
            const hideNow = () => {
              if (hidden || seq !== freezeSeq) return;
              hidden = true;
              void api.fstoEmbedHide();
            };
            const timer = window.setTimeout(hideNow, 400);
            if (covered) {
              api
                .fstoEmbedSnapshot()
                .then((url) => {
                  if (alive && seq === freezeSeq && url) setFrozen(url);
                })
                .catch(() => {})
                .finally(() => {
                  window.clearTimeout(timer);
                  hideNow();
                });
            } else {
              window.clearTimeout(timer);
              hideNow();
            }
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
          freezeSeq++;
          useFstoEmbedStore.getState().setVisible(true);
          last = key;
          api
            .fstoEmbedShow(b, takePendingUrl())
            .then(() => {
              if (!alive) return;
              setReady(true);
              // La página real ya está encima: fuera la foto.
              setFrozen(null);
            })
            .catch(() => {
              visible = false;
              useFstoEmbedStore.getState().setVisible(false);
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
    // Ya a la vista y llega una página pendiente: se vuelve a mostrar con ella.
    const offUrl = useFstoEmbedStore.subscribe((s, prev) => {
      if (s.pendingUrl && s.pendingUrl !== prev.pendingUrl && visible) {
        visible = false;
        sync();
      }
    });
    return () => {
      alive = false;
      offUrl();
      cancelAnimationFrame(raf);
      ro.disconnect();
      mo.disconnect();
      window.removeEventListener("resize", sync);
      useFstoEmbedStore.getState().setVisible(false);
      void api.fstoEmbedHide();
    };
  }, [epoch]);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-surface">
      <div ref={hostRef} className="relative min-h-0 flex-1">
        {frozen && (
          <img src={frozen} alt="" draggable={false} className="pointer-events-none absolute inset-0 h-full w-full select-none" />
        )}
        {isTauri ? (
          !ready && !frozen && (
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

/** (v8.3) Único control de flightsim.to: recargar. Va en la esquina de la
 *  cabecera de Explorar porque un botón HTML encima del hueco quedaría tapado
 *  por el webview nativo. */
export function FlightsimReloadButton() {
  return (
    <button
      onClick={() => void api.fstoEmbedNav("reload")}
      title={t("fsto.reload")}
      aria-label={t("fsto.reload")}
      className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full border border-sf-border bg-sf-surface text-sf-muted transition-colors hover:border-brand-500/40 hover:text-sf-text"
    >
      <RotateCw className="h-4 w-4" />
    </button>
  );
}
