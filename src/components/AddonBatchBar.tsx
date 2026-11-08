import { useState, type FormEvent } from "react";
import { Tag, X } from "lucide-react";
import { packageTags } from "../lib/addonGroups";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import type { AddonUserType, CommunityPackage } from "../lib/types";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useToastStore } from "../stores/useToastStore";

const USER_TYPES: { value: AddonUserType; label: string }[] = [
  { value: "AIRCRAFT", label: "addons.type.aircraft" },
  { value: "LIVERY", label: "addons.type.livery" },
  { value: "INSTRUMENT", label: "addons.type.instrument" },
  { value: "MISC", label: "addons.type.misc" },
];
const AUTO = "auto";

/** (06/11) Editar en lote lo seleccionado. Se guarda en SimFleet: nunca toca
 *  archivos del addon. */
export function AddonBatchBar({
  selected,
  visible,
  onChange,
  onClose,
}: {
  selected: Set<string>;
  visible: CommunityPackage[];
  onChange: (next: Set<string>) => void;
  onClose: () => void;
}) {
  const packages = useCommunityStore((s) => s.packages);
  const [tag, setTag] = useState("");
  const [busy, setBusy] = useState(false);
  const folders = [...selected];
  const tags = [
    ...new Set(packages.filter((p) => selected.has(p.folderName)).flatMap(packageTags)),
  ].sort((a, b) => a.localeCompare(b));
  const empty = folders.length === 0 || busy;

  const save = async (work: () => Promise<void>) => {
    setBusy(true);
    try {
      await work();
      useCommunityStore.setState({ packages: await api.listCommunityPackages() });
    } catch (e) {
      useToastStore.getState().push({ kind: "error", title: t("addons.select.error"), message: String(e) });
    } finally {
      setBusy(false);
    }
  };
  const addTag = (e: FormEvent) => {
    e.preventDefault();
    const name = tag.trim();
    if (!name || empty) return;
    setTag("");
    void save(() => api.editAddonsTags(folders, [name], []));
  };

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-xl border border-brand-500/40 bg-brand-500/10 px-3 py-2 text-xs text-sf-text">
      <span className="font-semibold">{t("addons.select.count", { n: folders.length })}</span>
      <button onClick={() => onChange(new Set(visible.map((p) => p.folderName)))} className="text-sf-muted hover:text-sf-text hover:underline">
        {t("addons.select.all")}
      </button>
      <button onClick={() => onChange(new Set())} className="text-sf-muted hover:text-sf-text hover:underline">
        {t("addons.select.none")}
      </button>
      <span className="h-4 w-px bg-sf-border" aria-hidden />
      <select
        value=""
        disabled={empty}
        aria-label={t("addons.select.type")}
        onChange={(e) => {
          const value = e.target.value;
          if (value) void save(() => api.setAddonsUserType(folders, value === AUTO ? null : (value as AddonUserType)));
        }}
        className="rounded-lg border border-sf-border bg-sf-surface px-2 py-1 text-xs text-sf-text disabled:opacity-50"
      >
        <option value="">{t("addons.select.type")}</option>
        {USER_TYPES.map((type) => (
          <option key={type.value} value={type.value}>
            {t(type.label)}
          </option>
        ))}
        <option value={AUTO}>{t("addons.select.type_auto")}</option>
      </select>
      <form onSubmit={addTag} className="flex items-center gap-1">
        <Tag className="h-3.5 w-3.5 text-sf-muted" />
        <input
          value={tag}
          onChange={(e) => setTag(e.target.value)}
          maxLength={32}
          placeholder={t("addons.select.tag_placeholder")}
          aria-label={t("addons.select.tag_placeholder")}
          className="w-32 rounded-lg border border-sf-border bg-sf-surface px-2 py-1 text-xs text-sf-text placeholder:text-sf-muted"
        />
        <button
          type="submit"
          disabled={empty || !tag.trim()}
          className="rounded-lg bg-brand-500 px-2 py-1 font-semibold text-slate-950 hover:bg-brand-400 disabled:opacity-50"
        >
          {t("addons.select.tag_add")}
        </button>
      </form>
      {tags.map((name) => (
        <button
          key={name}
          disabled={empty}
          onClick={() => void save(() => api.editAddonsTags(folders, [], [name]))}
          title={t("addons.select.tag_remove", { tag: name })}
          className="inline-flex items-center gap-1 rounded-full bg-sf-raised px-2 py-0.5 text-[11px] text-sf-text hover:bg-rose-500/15 disabled:opacity-50"
        >
          {name}
          <X className="h-3 w-3" />
        </button>
      ))}
      <button onClick={onClose} className="ml-auto rounded-lg border border-sf-border px-2.5 py-1 font-medium hover:bg-sf-raised">
        {t("addons.select.done")}
      </button>
    </div>
  );
}
