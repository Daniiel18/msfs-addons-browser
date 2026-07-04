import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Check, Copy, Download, Loader2, X } from "lucide-react";
import { t } from "../lib/i18n";
import {
  copyImageToClipboard,
  savePngToDisk,
  svgToPngBlob,
} from "../lib/shareImage";
import type { BuiltCard } from "../lib/shareCards";
import { useToastStore } from "../stores/useToastStore";

interface Props {
  title: string;
  card: BuiltCard;
  onClose: () => void;
}

/**
 * (v6.2.30 / R7) Shell reutilizable para mostrar una tarjeta SVG
 * compartible (vuelo o Wrapped) con acciones de copiar al portapapeles
 * y guardar como PNG. La rasterización vive en lib/shareImage.
 */
export function ShareCardModal({ title, card, onClose }: Props) {
  const pushToast = useToastStore((s) => s.push);
  const [copying, setCopying] = useState(false);
  const [copied, setCopied] = useState(false);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const doCopy = async () => {
    setCopying(true);
    try {
      const blob = await svgToPngBlob(card.svg, card.width, card.height);
      const ok = await copyImageToClipboard(blob);
      if (ok) {
        setCopied(true);
        setTimeout(() => setCopied(false), 1800);
      } else {
        pushToast({ kind: "error", title: t("share.copy_unsupported") });
      }
    } catch (e) {
      pushToast({ kind: "error", title: t("share.error"), message: String(e) });
    } finally {
      setCopying(false);
    }
  };

  const doSave = async () => {
    setSaving(true);
    try {
      const blob = await svgToPngBlob(card.svg, card.width, card.height);
      const res = await savePngToDisk(blob, card.defaultName);
      if (res === "saved") {
        pushToast({ kind: "success", title: t("share.saved"), ttlMs: 2200 });
      } else if (res === "error") {
        pushToast({ kind: "error", title: t("share.error") });
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <AnimatePresence>
      <motion.div
        key="share-backdrop"
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        onClick={onClose}
        className="fixed inset-0 z-[70] flex items-center justify-center bg-slate-950/75 p-4 backdrop-blur-sm"
      >
        <motion.div
          key="share-modal"
          initial={{ opacity: 0, scale: 0.96, y: 8 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={{ opacity: 0, scale: 0.96, y: 8 }}
          transition={{ duration: 0.16 }}
          onClick={(e) => e.stopPropagation()}
          className="relative flex max-h-[calc(100vh-3rem)] w-[min(460px,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
        >
          <header className="flex items-center justify-between gap-2 border-b border-slate-800 px-5 py-3">
            <h2 className="text-sm font-semibold text-slate-100">{title}</h2>
            <button
              onClick={onClose}
              className="rounded-md p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
            >
              <X className="h-4 w-4" />
            </button>
          </header>

          <div className="flex-1 overflow-y-auto p-4">
            <div
              className="mx-auto w-full max-w-[360px] overflow-hidden rounded-xl [&>svg]:h-auto [&>svg]:w-full"
              dangerouslySetInnerHTML={{ __html: card.svg }}
            />
          </div>

          <footer className="flex items-center gap-2 border-t border-slate-800 px-4 py-3">
            <button
              onClick={() => void doCopy()}
              disabled={copying}
              className="inline-flex flex-1 items-center justify-center gap-2 rounded-lg bg-brand-500 px-3 py-2 text-sm font-semibold text-white hover:bg-brand-400 disabled:opacity-50"
            >
              {copying ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : copied ? (
                <Check className="h-4 w-4" />
              ) : (
                <Copy className="h-4 w-4" />
              )}
              {copied ? t("share.copied") : t("share.copy")}
            </button>
            <button
              onClick={() => void doSave()}
              disabled={saving}
              className="inline-flex items-center justify-center gap-2 rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm font-medium text-slate-200 hover:bg-slate-800 disabled:opacity-50"
            >
              {saving ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Download className="h-4 w-4" />
              )}
              {t("share.save")}
            </button>
          </footer>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}
