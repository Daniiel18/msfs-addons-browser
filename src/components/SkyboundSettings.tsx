import { useEffect, useState } from "react";
import { Check, Loader2, LogIn } from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * (v6.2.38) Credenciales de Skybound (catálogo MSFS2024). El usuario las
 * guarda aquí; el backend mantiene la sesión. Sólo es útil en MSFS2024.
 */
export function SkyboundSettings() {
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [saving, setSaving] = useState(false);
  const [status, setStatus] = useState<{ hasCredentials: boolean; loggedIn: boolean } | null>(
    null,
  );

  const refresh = () => {
    api.skyboundStatus().then(setStatus).catch(() => setStatus(null));
  };
  useEffect(refresh, []);

  const save = async () => {
    setSaving(true);
    try {
      await api.skyboundSetCredentials(user.trim(), pass);
      setPass("");
      refresh();
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="text-xs text-slate-200">{t("settings.skybound.title")}</div>
      <p className="mt-0.5 text-[11px] leading-snug text-slate-500">
        {t("settings.skybound.hint")}
      </p>
      <div className="mt-2 grid gap-2 sm:grid-cols-2">
        <input
          value={user}
          onChange={(e) => setUser(e.target.value)}
          placeholder={t("settings.skybound.user")}
          autoComplete="off"
          className="rounded-md border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-xs text-slate-100 placeholder:text-slate-600 focus:border-brand-500/50 focus:outline-none"
        />
        <input
          value={pass}
          onChange={(e) => setPass(e.target.value)}
          type="password"
          placeholder={t("settings.skybound.pass")}
          autoComplete="new-password"
          className="rounded-md border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-xs text-slate-100 placeholder:text-slate-600 focus:border-brand-500/50 focus:outline-none"
        />
      </div>
      <div className="mt-2 flex items-center gap-2">
        <button
          onClick={() => void save()}
          disabled={saving || !user.trim() || !pass}
          className="inline-flex items-center gap-1.5 rounded-md bg-brand-500 px-3 py-1.5 text-xs font-semibold text-white hover:bg-brand-400 disabled:opacity-50"
        >
          {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <LogIn className="h-3.5 w-3.5" />}
          {t("settings.skybound.save")}
        </button>
        {status?.hasCredentials && (
          <span className="inline-flex items-center gap-1 text-[11px] text-emerald-300">
            <Check className="h-3 w-3" />
            {t("settings.skybound.configured")}
          </span>
        )}
      </div>
    </div>
  );
}
