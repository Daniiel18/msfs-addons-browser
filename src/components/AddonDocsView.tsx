import { useCallback, useEffect, useMemo, useState } from "react";
import { FileText, FolderOpen, Loader2, Search } from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import type { AddonDoc } from "../lib/types";
import { DocPreviewModal } from "./DocPreviewModal";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useToastStore } from "../stores/useToastStore";

function fmtSize(bytes: number): string {
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

function useAddonDocs(): AddonDoc[] | null {
  const [docs, setDocs] = useState<AddonDoc[] | null>(null);
  useEffect(() => {
    let alive = true;
    api
      .addonDocs()
      .then((list) => alive && setDocs(list))
      .catch((e) => {
        console.warn("[docs]", e);
        if (alive) setDocs([]);
      });
    return () => {
      alive = false;
    };
  }, []);
  return docs;
}

function DocRow({ doc }: { doc: AddonDoc }) {
  const [preview, setPreview] = useState(false);
  const closePreview = useCallback(() => setPreview(false), []);
  // La web y los formatos de oficina se abren fuera, como siempre.
  const open = () => {
    if (doc.kind === "pdf" || doc.kind === "text") {
      setPreview(true);
      return;
    }
    void api
      .openLocalPath(doc.path)
      .catch((e) =>
        useToastStore.getState().push({ kind: "error", title: t("docs.open_error"), message: String(e) }),
      );
  };
  return (
    <li className="flex items-center gap-1 rounded-lg hover:bg-sf-raised/60">
      {preview && <DocPreviewModal doc={doc} onClose={closePreview} />}
      <button type="button" onClick={open} title={doc.path} className="flex min-w-0 flex-1 items-center gap-2 px-2 py-1.5 text-left">
        <FileText className="h-4 w-4 shrink-0 text-sf-muted" />
        <span className="min-w-0 flex-1 truncate text-xs text-sf-text">{doc.name}</span>
        <span className="shrink-0 text-[11px] text-sf-muted">
          {t(`docs.kind.${doc.kind}`)} · {fmtSize(doc.sizeBytes)}
        </span>
      </button>
      <button
        type="button"
        onClick={() => void api.revealPath(doc.path, true)}
        title={t("docs.open_folder")}
        aria-label={t("docs.open_folder")}
        className="mr-1 shrink-0 rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
      >
        <FolderOpen className="h-3.5 w-3.5" />
      </button>
    </li>
  );
}

/**
 * (04/11) Etapa 9 — vista «Documentos» de Instalados: manuales, guías y notas
 * que traen los addons, agrupados por addon.
 */
export function AddonDocsView() {
  const docs = useAddonDocs();
  const packages = useCommunityStore((s) => s.packages);
  const [q, setQ] = useState("");

  const groups = useMemo(() => {
    const titles = new Map(packages.map((p) => [p.folderName, p.airportName ?? p.title]));
    const term = q.trim().toLowerCase();
    const byAddon = new Map<string, { title: string; docs: AddonDoc[] }>();
    for (const d of docs ?? []) {
      const title = titles.get(d.folder) ?? d.folder;
      if (term && !`${d.name} ${title} ${d.folder}`.toLowerCase().includes(term)) continue;
      const group = byAddon.get(d.folder) ?? { title, docs: [] };
      group.docs.push(d);
      byAddon.set(d.folder, group);
    }
    return [...byAddon.entries()].sort(([, a], [, b]) => a.title.localeCompare(b.title));
  }, [docs, packages, q]);

  if (docs === null) {
    return (
      <p className="flex items-center justify-center gap-2 py-16 text-sm text-sf-muted">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("docs.loading")}
      </p>
    );
  }
  const shown = groups.reduce((n, [, g]) => n + g.docs.length, 0);

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-3">
        <label className="relative min-w-[16rem] flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-sf-muted" />
          <input
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder={t("docs.search")}
            aria-label={t("docs.search")}
            className="w-full rounded-xl border border-sf-border bg-sf-surface py-2 pl-9 pr-3 text-sm text-sf-text outline-none placeholder:text-sf-muted focus:border-brand-500"
          />
        </label>
        <span className="text-xs text-sf-muted">{t("docs.count", { docs: shown, addons: groups.length })}</span>
      </div>
      {groups.length === 0 ? (
        <p className="py-16 text-center text-sm text-sf-muted">{t(docs.length === 0 ? "docs.empty" : "docs.no_match")}</p>
      ) : (
        <div className="grid items-start gap-3 lg:grid-cols-2">
          {groups.map(([folder, group]) => (
            <section key={folder} className="min-w-0 rounded-xl border border-sf-border bg-sf-surface p-2">
              <h3 className="truncate px-2 py-1 text-xs font-semibold text-sf-text" title={folder}>
                {group.title}
              </h3>
              <ul>
                {group.docs.map((d) => (
                  <DocRow key={d.path} doc={d} />
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

/** Los documentos de un addon, en su ficha. No ocupa nada si no tiene. */
export function AddonDocsSection({ folder }: { folder: string }) {
  const docs = useAddonDocs();
  const mine = useMemo(() => (docs ?? []).filter((d) => d.folder === folder), [docs, folder]);
  if (mine.length === 0) return null;
  return (
    <section className="mt-4">
      <h3 className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">
        {t("docs.section")} ({mine.length})
      </h3>
      <ul>
        {mine.map((d) => (
          <DocRow key={d.path} doc={d} />
        ))}
      </ul>
    </section>
  );
}
