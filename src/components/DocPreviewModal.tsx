import { useEffect, useMemo, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ExternalLink, Loader2, X } from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { parseMarkdown, type MdBlock, type MdInline } from "../lib/miniMarkdown";
import type { AddonDoc, AddonDocPreview } from "../lib/types";

function Inline({ parts }: { parts: MdInline[] }) {
  return (
    <>
      {parts.map((p, i) => {
        if (p.t === "bold") return <strong key={i} className="font-semibold text-sf-text">{p.v}</strong>;
        if (p.t === "em") return <em key={i}>{p.v}</em>;
        if (p.t === "code") return <code key={i} className="rounded bg-sf-raised px-1 font-mono text-[11px]">{p.v}</code>;
        if (p.t === "link") {
          return (
            <button
              key={i}
              type="button"
              onClick={() => void api.openExternal(p.href)}
              title={p.href}
              className="text-brand-300 underline hover:text-brand-200"
            >
              {p.v}
            </button>
          );
        }
        return <span key={i}>{p.v}</span>;
      })}
    </>
  );
}

function Block({ block }: { block: MdBlock }): ReactNode {
  switch (block.t) {
    case "h": {
      const size = ["text-lg", "text-base", "text-sm"][block.level - 1];
      return <h3 className={`mt-4 font-semibold text-sf-text first:mt-0 ${size}`}><Inline parts={block.text} /></h3>;
    }
    case "p":
      return <p className="mt-2 leading-relaxed"><Inline parts={block.text} /></p>;
    case "ul":
    case "ol": {
      const List = block.t;
      const style = block.t === "ul" ? "list-disc" : "list-decimal";
      return (
        <List className={`mt-2 space-y-1 pl-5 ${style}`}>
          {block.items.map((item, i) => (
            <li key={i}><Inline parts={item} /></li>
          ))}
        </List>
      );
    }
    case "quote":
      return <blockquote className="mt-2 border-l-2 border-sf-border pl-3 italic"><Inline parts={block.text} /></blockquote>;
    case "code":
      return <pre className="mt-2 overflow-x-auto rounded-lg bg-sf-raised/60 p-2 font-mono text-[11px] text-sf-text">{block.v}</pre>;
    case "hr":
      return <hr className="my-3 border-sf-border" />;
  }
}

/**
 * (07/11) Etapa F — vista previa de un documento dentro de SimFleet: el PDF con
 * el visor de WebView2 y el texto o Markdown aquí mismo. Lo que no se puede
 * mostrar se abre con su programa, como antes.
 */
export function DocPreviewModal({ doc, onClose }: { doc: AddonDoc; onClose: () => void }) {
  const [preview, setPreview] = useState<AddonDocPreview | null>(null);

  const openOutside = () => {
    void api.openLocalPath(doc.path).catch(() => undefined);
    onClose();
  };

  useEffect(() => {
    let alive = true;
    api
      .addonDocPreview(doc.path)
      .then((p) => {
        if (!alive) return;
        if (p.kind === "external") openOutside();
        else setPreview(p);
      })
      .catch(() => alive && openOutside());
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [doc.path]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const blocks = useMemo(() => (preview?.kind === "markdown" ? parseMarkdown(preview.text ?? "") : []), [preview]);

  return createPortal(
    <div
      className="fixed inset-0 z-[80] flex items-center justify-center bg-slate-950/70 p-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="flex h-[88vh] w-[min(980px,100%)] flex-col overflow-hidden rounded-2xl border border-sf-border bg-sf-surface shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 border-b border-sf-border px-4 py-2.5">
          <p className="min-w-0 flex-1 truncate text-sm font-semibold text-sf-text" title={doc.path}>
            {doc.name}
          </p>
          <button
            type="button"
            onClick={openOutside}
            className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-sf-border px-2.5 py-1 text-xs font-medium text-sf-text hover:bg-sf-raised"
          >
            <ExternalLink className="h-3.5 w-3.5" />
            {t("docs.preview.external")}
          </button>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("docs.preview.close")}
            className="shrink-0 rounded-md p-1.5 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        {!preview && (
          <p className="flex flex-1 items-center justify-center gap-2 text-sm text-sf-muted">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("docs.preview.loading")}
          </p>
        )}
        {preview?.kind === "pdf" && (
          <iframe title={doc.name} src={convertFileSrc(doc.path)} className="min-h-0 flex-1 bg-sf-bg" />
        )}
        {preview?.kind === "text" && (
          <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-words p-4 font-mono text-xs text-sf-text">
            {preview.text}
          </pre>
        )}
        {preview?.kind === "markdown" && (
          <div className="min-h-0 flex-1 overflow-auto p-5 text-xs text-sf-muted">
            {blocks.map((b, i) => (
              <Block key={i} block={b} />
            ))}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}
