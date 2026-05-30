import { useMemo } from "react";
import { Download } from "lucide-react";
import { useDownloadsStore } from "../stores/useDownloadsStore";

export function DownloadsButton() {
  const jobsMap = useDownloadsStore((s) => s.jobs);
  const toggle = useDownloadsStore((s) => s.togglePanel);

  const active = useMemo(() => {
    let count = 0;
    for (const j of Object.values(jobsMap)) {
      if (
        j.phase === "queued" ||
        j.phase === "resolving" ||
        j.phase === "downloading" ||
        j.phase === "installing"
      ) {
        count++;
      }
    }
    return count;
  }, [jobsMap]);

  return (
    <button
      onClick={toggle}
      className="relative inline-flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900 px-3 py-1.5 text-xs font-medium text-slate-300 hover:border-brand-500/40 hover:text-slate-100"
      title="Descargas"
    >
      <Download className="h-3.5 w-3.5" />
      <span>Descargas</span>
      {active > 0 && (
        <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-brand-500 px-1 text-[10px] font-bold text-slate-950">
          {active}
        </span>
      )}
    </button>
  );
}
