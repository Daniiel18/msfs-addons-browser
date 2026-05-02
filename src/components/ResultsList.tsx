import { AnimatePresence, motion } from "framer-motion";
import { AlertCircle, Inbox, Loader2 } from "lucide-react";
import type { Addon } from "../lib/types";
import { ResultCard } from "./ResultCard";

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
        <p className="mt-3 text-sm text-slate-400">
          {browseMode === "browse"
            ? "Cargando catálogo…"
            : "Buscando en la fuente seleccionada…"}
        </p>
      </CenterState>
    );
  }

  if (status === "error") {
    return (
      <CenterState>
        <AlertCircle className="h-8 w-8 text-red-400" />
        <p className="mt-3 text-sm font-medium text-slate-200">Algo salió mal</p>
        <p className="text-xs text-slate-500">{error ?? "Error desconocido"}</p>
      </CenterState>
    );
  }

  if (status === "idle") {
    return (
      <CenterState>
        <Loader2 className="h-8 w-8 animate-spin text-slate-600" />
        <p className="mt-3 text-sm text-slate-400">Cargando catálogo…</p>
      </CenterState>
    );
  }

  if (status === "success" && results.length === 0) {
    return (
      <CenterState>
        <Inbox className="h-8 w-8 text-slate-600" />
        <p className="mt-3 text-sm text-slate-400">
          {browseMode === "browse"
            ? "El catálogo no devolvió resultados en esta página."
            : (
              <>
                Sin resultados para{" "}
                <span className="text-slate-200">“{query}”</span>.
              </>
            )}
        </p>
      </CenterState>
    );
  }

  return (
    <motion.div layout className="grid gap-3">
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
    <div className="flex flex-col items-center justify-center rounded-2xl border border-dashed border-slate-800 bg-slate-900/30 px-6 py-16 text-center">
      {children}
    </div>
  );
}
