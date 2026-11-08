import { useState } from "react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { conditionsText, parseCodes } from "../lib/linkConditions";
import type { AddonLink, LinkConditions } from "../lib/types";

const FIELDS: { key: keyof LinkConditions; example: string }[] = [
  { key: "country", example: "BR, CA" },
  { key: "airport", example: "SBGR, CYOW" },
  { key: "aircraft", example: "B738, A20N" },
  { key: "airline", example: "WJA, AVA" },
];

/** (05/11) En qué vuelos vale un enlace manual del Link Map. */
export function LinkConditionEditor({
  link,
  onSaved,
  onCancel,
}: {
  link: AddonLink;
  onSaved: () => Promise<void> | void;
  onCancel: () => void;
}) {
  const [text, setText] = useState<Record<keyof LinkConditions, string>>(() => ({
    country: link.conditions?.country.join(", ") ?? "",
    airport: link.conditions?.airport.join(", ") ?? "",
    aircraft: link.conditions?.aircraft.join(", ") ?? "",
    airline: link.conditions?.airline.join(", ") ?? "",
  }));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const draft: LinkConditions = {
    country: parseCodes(text.country),
    airport: parseCodes(text.airport),
    aircraft: parseCodes(text.aircraft),
    airline: parseCodes(text.airline),
  };
  const filled = Object.values(draft).some((list) => list.length > 0);

  const save = async (conditions: LinkConditions | null) => {
    setBusy(true);
    setError(null);
    try {
      await api.setAddonLinkConditions(link.sourceFolder, link.targetFolder, conditions);
      await onSaved();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="mt-2 space-y-2 border-t border-sf-border pt-2">
      <p className="text-[11px] leading-snug text-sf-muted">{t("linkmap.cond.intro")}</p>
      <div className="grid grid-cols-2 gap-2">
        {FIELDS.map(({ key, example }) => (
          <label key={key} className="block text-[11px] text-sf-muted">
            {t(`linkmap.cond.${key}`)}
            <input
              value={text[key]}
              onChange={(e) => {
                setText({ ...text, [key]: e.target.value });
                setError(null);
              }}
              placeholder={example}
              spellCheck={false}
              className="mt-0.5 w-full rounded-md border border-sf-border bg-sf-bg px-2 py-1 font-mono text-[11px] uppercase text-sf-text placeholder:normal-case focus:border-violet-400 focus:outline-none"
            />
          </label>
        ))}
      </div>
      <p className="text-[11px] font-medium text-violet-300">
        {filled ? conditionsText(draft) : t("linkmap.cond.none")}
      </p>
      {error && <p className="text-[11px] text-rose-300">{error}</p>}
      <div className="flex justify-end gap-2">
        {link.conditions && (
          <button
            onClick={() => void save(null)}
            disabled={busy}
            className="mr-auto rounded-md px-2 py-1 text-[11px] text-rose-300 hover:bg-rose-500/15 disabled:opacity-50"
          >
            {t("linkmap.cond.remove")}
          </button>
        )}
        <button onClick={onCancel} className="rounded-md px-2 py-1 text-[11px] text-sf-muted hover:bg-sf-raised hover:text-sf-text">
          {t("linkmap.cond.cancel")}
        </button>
        <button
          onClick={() => void save(filled ? draft : null)}
          disabled={busy}
          className="rounded-md bg-violet-500 px-2.5 py-1 text-[11px] font-semibold text-white hover:bg-violet-400 disabled:opacity-50"
        >
          {t("linkmap.cond.save")}
        </button>
      </div>
    </div>
  );
}
