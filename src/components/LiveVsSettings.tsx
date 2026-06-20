import { useState } from "react";
import { Users, ChevronDown, CheckCircle2 } from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useSettingsStore } from "../stores/useSettingsStore";

/**
 * (v6 #3) Ajustes de "Live VS" — credenciales de Supabase Realtime para el
 * modo competitivo Daniel vs Héctor. La URL + anon key se guardan en settings
 * (NO en código). Cuando ambos campos están, el canal Realtime se activa.
 */
export function LiveVsSettings() {
  const [open, setOpen] = useState(false);
  const settings = useSettingsStore((s) => s.settings);
  const bootstrap = useSettingsStore((s) => s.bootstrap);
  const [url, setUrl] = useState(settings.vsSupabaseUrl ?? "");
  const [key, setKey] = useState(settings.vsSupabaseKey ?? "");

  const configured =
    (settings.vsSupabaseUrl ?? "").trim() !== "" &&
    (settings.vsSupabaseKey ?? "").trim() !== "";

  const save = async (k: string, v: string) => {
    await api.setAppSetting(k, v.trim()).catch(() => {});
    await bootstrap();
  };

  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between gap-2 text-left"
        aria-expanded={open}
      >
        <span className="flex items-center gap-2">
          <Users className="h-4 w-4 text-brand-400" />
          <span>
            <span className="flex items-center gap-1.5 text-xs text-slate-200">
              {t("vs.title")}
              {configured && (
                <CheckCircle2 className="h-3.5 w-3.5 text-brand-400" />
              )}
            </span>
            <span className="mt-0.5 block text-[11px] text-slate-500">
              {t("vs.subtitle")}
            </span>
          </span>
        </span>
        <ChevronDown
          className={`h-4 w-4 text-slate-500 transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open && (
        <div className="mt-3 space-y-3">
          <div>
            <div className="mb-1 text-[11px] text-slate-500">
              {t("vs.url")}
            </div>
            <input
              type="text"
              value={url}
              placeholder="https://xxxx.supabase.co"
              spellCheck={false}
              onChange={(e) => setUrl(e.target.value)}
              onBlur={() => save("vs_supabase_url", url)}
              className="w-full rounded-md border border-slate-800 bg-slate-950/40 px-2 py-1.5 font-mono text-[11px] text-slate-200 focus:border-brand-500/50 focus:outline-none"
            />
          </div>
          <div>
            <div className="mb-1 text-[11px] text-slate-500">
              {t("vs.key")}
            </div>
            <input
              type="password"
              value={key}
              placeholder="eyJhbGci…"
              spellCheck={false}
              onChange={(e) => setKey(e.target.value)}
              onBlur={() => save("vs_supabase_key", key)}
              className="w-full rounded-md border border-slate-800 bg-slate-950/40 px-2 py-1.5 font-mono text-[11px] text-slate-200 focus:border-brand-500/50 focus:outline-none"
            />
          </div>
          <p className="text-[11px] leading-relaxed text-slate-600">
            {t("vs.note")}
          </p>
        </div>
      )}
    </div>
  );
}
