import { AnimatePresence, motion } from "framer-motion";
import { AlertCircle, Inbox, Loader2 } from "lucide-react";
import type { Addon } from "../lib/types";
import { ResultCard } from "./ResultCard";
import { t } from "../lib/i18n";

interface Props {
  results: Addon[];
  status: "idle" | "loading" | "success" | "error";
  error: string | null;
  query: string;
  /** Cuando es "browse", los resultados son del catálogo paginado
   *  (sin query). Cambia los textos vacíos para no decir "sin
   *  resultados para ''". */
  browseMode?: "search" | "browse";
}

export function ResultsList({
  results,
  status,
  error,
  query,
  browseMode = "search",
}: Props) {
  if (status === "loading") {
    return (
      <CenterState>
        <Loader2 className="h-8 w-8 animate-spin text-brand-400" />
        <p className="mt-3 text-sm text-sf-muted">
          {browseMode === "browse"
            ? t("results.loading_catalog")
            : t("results.searching_source")}
        </p>
      </CenterState>
    );
  }

  if (status === "error") {
    return (
      <CenterState>
        <AlertCircle className="h-8 w-8 text-red-400" />
        <p className="mt-3 text-sm font-medium text-sf-text">{t("results.error_title")}</p>
        <p className="text-xs text-sf-muted">{error ?? t("results.unknown_error")}</p>
      </CenterState>
    );
  }

  if (status === "idle") {
    return (
      <CenterState>
        <Loader2 className="h-8 w-8 animate-spin text-sf-muted/70" />
        <p className="mt-3 text-sm text-sf-muted">{t("results.loading_catalog")}</p>
      </CenterState>
    );
  }

  if (status === "success" && results.length === 0) {
    return (
      <CenterState>
        <Inbox className="h-8 w-8 text-sf-muted/70" />
        <p className="mt-3 text-sm text-sf-muted">
          {browseMode === "browse"
            ? t("results.empty_browse")
            : (
              <>
                {t("results.empty_search_prefix")}{" "}
                <span className="text-sf-text">"{query}"</span>.
              </>
            )}
        </p>
      </CenterState>
    );
  }

  return (
    /* (Rediseño F3) Rejilla que se adapta al ancho disponible: 2 columnas
       en ventanas medianas, 3–4 en anchas (antes 1 resultado por fila). */
    <motion.div layout className="grid grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-4">
      <AnimatePresence initial={false}>
        {results.map((addon) => (
          <ResultCard key={addon.id} addon={addon} />
        ))}
      </AnimatePresence>
    </motion.div>
  );
}

function CenterState({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center rounded-xl border border-dashed border-sf-border bg-sf-surface/50 px-6 py-16 text-center">
      {children}
    </div>
  );
}
