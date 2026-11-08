import { TriangleAlert } from "lucide-react";
import { sceneriesFor } from "../lib/dispatch";
import { t } from "../lib/i18n";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useDispatchStore } from "../stores/useDispatchStore";

/** (05/11) Aviso en la ficha de un aeropuerto con varios escenarios encendidos:
 *  se elige cuál usar al lanzar o se deja de avisar. */
export function SceneryClashNote({ icao }: { icao: string | null }) {
  const packages = useCommunityStore((s) => s.packages);
  const choices = useDispatchStore((s) => s.sceneryChoices);
  const choose = useDispatchStore((s) => s.setSceneryChoice);
  if (!icao) return null;
  const code = icao.toUpperCase();
  const lit = sceneriesFor(code, packages).filter((p) => p.enabled !== false);
  if (lit.length < 2) return null;
  const choice = choices.find((c) => c.icao === code);
  if (choice?.ignored) {
    return (
      <button
        onClick={() => void choose(code, null, false)}
        className="mx-4 mb-2 text-[11px] text-sf-muted underline-offset-2 hover:text-sf-text hover:underline"
      >
        {t("map.clash.warn_again")}
      </button>
    );
  }
  return (
    <div className="mx-4 mb-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-100">
      <p className="flex items-start gap-1.5 font-medium">
        <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
        {t("map.clash.title", { icao: code })}
      </p>
      <div className="mt-1.5 flex flex-col gap-1">
        {lit.map((p) => (
          <button
            key={p.folderName}
            onClick={() => void choose(code, p.folderName, false)}
            title={p.folderName}
            className={`truncate rounded-md border px-2 py-1 text-left ${
              choice?.folder === p.folderName
                ? "border-amber-400 bg-amber-500/20 text-amber-50"
                : "border-amber-500/30 hover:bg-amber-500/15"
            }`}
          >
            {p.title}
            {p.creator ? ` · ${p.creator}` : ""}
          </button>
        ))}
      </div>
      <p className="mt-1.5 text-[11px] leading-snug text-amber-200/80">{t("map.clash.hint")}</p>
      <button
        onClick={() => void choose(code, null, true)}
        className="mt-1 text-[11px] underline-offset-2 hover:underline"
      >
        {t("map.clash.ignore")}
      </button>
    </div>
  );
}
