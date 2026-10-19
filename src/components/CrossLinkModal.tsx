import { useState } from "react";
import { motion } from "framer-motion";
import { Link2, Loader2, FolderSymlink, X } from "lucide-react";
import type { CrossLinkOffer } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useToastStore } from "../stores/useToastStore";

/**
 * (v6 #1) Modal post-instalación de CROSS-LINK 2020/2024.
 *
 * Se abre cuando el backend emite `cross-link://offer` — es decir, cuando el
 * usuario tiene MSFS 2020 Y 2024 instalados y acaba de descargar/instalar un
 * escenario rotulado "MSFS 2020/2024". Ofrece crear un junction NTFS en la
 * Community de la OTRA versión para que el mismo paquete aparezca en ambos
 * sims SIN duplicar espacio en disco.
 *
 * A diferencia del What's New, este modal SÍ es descartable (backdrop / "Ahora
 * no" / X): es una oferta, no algo obligatorio.
 */
export function CrossLinkModal({
  offer,
  onClose,
}: {
  offer: CrossLinkOffer;
  onClose: () => void;
}) {
  const [linking, setLinking] = useState(false);
  const push = useToastStore((s) => s.push);

  const doLink = async () => {
    if (linking) return;
    setLinking(true);
    try {
      const res = await api.crossLinkCreate(offer.otherCommunity, offer.packages);
      const linked = res.linked.length;
      const skipped = res.skipped.length;
      const failed = res.failed.length;
      if (failed > 0) {
        push({
          kind: "error",
          title: t("crosslink.toast.failed", { n: String(failed) }),
          message: res.failed.join(" · "),
          ttlMs: 9000,
        });
      } else {
        push({
          kind: "success",
          title: t("crosslink.toast.done", {
            n: String(linked),
            label: offer.otherLabel,
          }),
          message:
            skipped > 0
              ? t("crosslink.toast.skipped", { n: String(skipped) })
              : undefined,
        });
      }
    } catch (e) {
      push({
        kind: "error",
        title: t("crosslink.toast.error"),
        message: String(e),
        ttlMs: 9000,
      });
    } finally {
      setLinking(false);
      onClose();
    }
  };

  return (
    <div
      className="fixed inset-0 z-[120] flex items-center justify-center bg-slate-950/80 px-4 backdrop-blur-sm"
      onClick={() => !linking && onClose()}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 10 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.2 }}
        onClick={(e) => e.stopPropagation()}
        className="w-[min(460px,calc(100vw-2rem))] overflow-hidden rounded-2xl border border-sf-border bg-sf-bg shadow-2xl ring-1 ring-sf-border"
      >
        <div className="flex items-start gap-3 border-b border-sf-border bg-sf-surface p-5">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-brand-500/15 ring-1 ring-brand-500/30">
            <Link2 className="h-5 w-5 text-brand-300" />
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="text-base font-semibold text-sf-text">
              {t("crosslink.title")}
            </h2>
            <p className="mt-1 text-sm leading-relaxed text-sf-muted">
              {t("crosslink.body", { label: offer.otherLabel })}
            </p>
          </div>
          <button
            onClick={() => !linking && onClose()}
            className="shrink-0 rounded-md p-1 text-sf-muted hover:text-sf-text/90"
            aria-label={t("common.close")}
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="p-5">
          {/* Lista de paquetes a enlazar */}
          <ul className="max-h-40 space-y-1 overflow-y-auto">
            {offer.packages.map((p) => (
              <li
                key={p.name}
                className="flex items-center gap-2 rounded-md bg-sf-surface px-3 py-1.5 text-xs text-sf-text/90"
              >
                <FolderSymlink className="h-3.5 w-3.5 shrink-0 text-brand-400" />
                <span className="truncate">{p.name}</span>
              </li>
            ))}
          </ul>

          {/* Destino + nota sobre el junction */}
          <p className="mt-3 truncate text-[11px] text-sf-muted" title={offer.otherCommunity}>
            → {offer.otherCommunity}
          </p>
          <p className="mt-1 text-[11px] leading-relaxed text-sf-muted">
            {t("crosslink.note")}
          </p>

          <div className="mt-5 flex items-center justify-end gap-2">
            <button
              onClick={() => !linking && onClose()}
              disabled={linking}
              className="rounded-md border border-sf-border bg-sf-surface px-4 py-2 text-xs text-sf-text/90 hover:border-slate-600 hover:text-sf-text disabled:opacity-40"
            >
              {t("crosslink.later")}
            </button>
            <button
              onClick={doLink}
              disabled={linking}
              className="inline-flex items-center gap-1.5 rounded-md bg-brand-500 px-4 py-2 text-xs font-medium text-slate-950 hover:bg-brand-400 disabled:opacity-60"
            >
              {linking ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t("crosslink.linking")}
                </>
              ) : (
                <>
                  <Link2 className="h-3.5 w-3.5" />
                  {t("crosslink.link")}
                </>
              )}
            </button>
          </div>
        </div>
      </motion.div>
    </div>
  );
}
