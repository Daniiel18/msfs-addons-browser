import { useMemo } from "react";
import { Download } from "lucide-react";
import { useDownloadsStore } from "../stores/useDownloadsStore";
import { t } from "../lib/i18n";

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

  const label = t("nav.downloads");
  return (
    <button
      onClick={toggle}
      className="relative inline-flex h-8 items-center gap-2 rounded-lg border border-sf-border bg-sf-surface px-3 text-xs font-medium text-sf-text/90 hover:border-brand-500/40 hover:text-sf-text"
      title={label}
    >
      <Download className="h-3.5 w-3.5" />
      <span>{label}</span>
      {active > 0 && (
        <span className="flex h-4 min-w-4 items-center justify-center rounded-full bg-brand-500 px-1 text-[11px] font-bold text-slate-950">
          {active}
        </span>
      )}
    </button>
  );
}
