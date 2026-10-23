import { useCallback, useEffect, useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import type { OpenSkyStatus } from "../lib/types";
import { useToastStore } from "../stores/useToastStore";

/** (v8.3 · Etapa 6.2a) Vuelos reales de OpenSky con la cuenta del usuario:
 *  estado de la conexión y de la descarga (ventanas de 2 h, créditos). Las
 *  claves van cifradas en la build; aquí no se escriben ni se muestran. */
export function OpenSkySetting() {
  const [st, setSt] = useState<OpenSkyStatus | null>(null);
  const [busy, setBusy] = useState(false);
  // (v8.3) Código de activación de las claves (lo comparte el dueño de la cuenta).
  const [code, setCode] = useState("");
  const pushToast = useToastStore((s) => s.push);

  const load = useCallback(() => {
    api
      .openskyStatus()
      .then(setSt)
      .catch(() => setSt(null));
  }, []);

  useEffect(() => {
    load();
    const id = window.setInterval(load, 10_000);
    return () => window.clearInterval(id);
  }, [load]);

  const syncNow = async () => {
    setBusy(true);
    try {
      const r = await api.openskySyncNow();
      pushToast({
        kind: r.stopped && r.windows === 0 ? "info" : "success",
        title: t("settings.opensky.synced", {
          windows: String(r.windows),
          flights: String(r.kept),
          credits: String(r.credits),
        }),
        message: r.stopped ? t("settings.opensky.stopped", { reason: stopLabel(r.stopped) }) : undefined,
      });
    } catch (e) {
      pushToast({ kind: "error", title: t("settings.opensky.error"), message: String(e) });
    } finally {
      setBusy(false);
      load();
    }
  };

  const activate = async () => {
    if (!code.trim()) return;
    setBusy(true);
    try {
      setSt(await api.openskySetActivation(code));
      setCode("");
      pushToast({ kind: "success", title: t("settings.opensky.activated"), ttlMs: 3000 });
    } catch (e) {
      pushToast({ kind: "error", title: t("settings.opensky.activate_error"), message: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const fmtTime = (iso: string | null) =>
    iso
      ? new Date(iso).toLocaleString([], {
          day: "2-digit",
          month: "short",
          hour: "2-digit",
          minute: "2-digit",
        })
      : "—";

  return (
    <div className="rounded-md border border-sf-border bg-sf-surface px-3 py-2.5">
      <div className="flex items-center gap-2 text-xs text-sf-text">
        <span
          className={`h-2 w-2 rounded-full ${st?.configured ? (st.lastError ? "bg-amber-400" : "bg-emerald-400") : "bg-slate-500"}`}
        />
        {st?.configured
          ? t("settings.opensky.connected")
          : st?.needsCode
            ? t("settings.opensky.needs_code")
            : t("settings.opensky.not_configured")}
        {(busy || st?.running) && <Loader2 className="h-3 w-3 animate-spin text-sf-muted" />}
        {st?.configured && (
          <button
            onClick={() => void syncNow()}
            disabled={busy || st.running}
            className="ml-auto inline-flex items-center gap-1.5 rounded-md bg-sf-raised px-2.5 py-1 text-[11px] font-semibold text-sf-text hover:bg-sf-border disabled:opacity-50"
          >
            <RefreshCw className="h-3 w-3" />
            {t("settings.opensky.sync_now")}
          </button>
        )}
      </div>
      <p className="mt-0.5 text-[11px] text-sf-muted">
        {st?.configured
          ? t("settings.opensky.hint")
          : st?.needsCode
            ? t("settings.opensky.needs_code_hint")
            : t("settings.opensky.not_configured_hint")}
      </p>
      {st?.needsCode && (
        <div className="mt-2 flex gap-2">
          <input
            value={code}
            onChange={(e) => setCode(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void activate();
            }}
            placeholder={t("settings.opensky.code_placeholder")}
            autoComplete="off"
            spellCheck={false}
            className="w-full rounded-md border border-sf-border bg-sf-raised px-2.5 py-1.5 font-mono text-xs text-sf-text placeholder:text-sf-muted/70 focus:border-brand-500/60 focus:outline-none"
          />
          <button
            onClick={() => void activate()}
            disabled={busy || !code.trim()}
            className="shrink-0 rounded-md bg-brand-500/80 px-3 py-1.5 text-[11px] font-semibold text-white hover:bg-brand-500 disabled:opacity-50"
          >
            {t("settings.opensky.activate")}
          </button>
        </div>
      )}
      {st?.configured && (
        <dl className="mt-2 grid grid-cols-[auto,1fr] gap-x-3 gap-y-1 text-[11px]">
          <dt className="text-sf-muted">{t("settings.opensky.last")}</dt>
          <dd className="text-sf-text">{fmtTime(st.lastFetchAt)}</dd>
          <dt className="text-sf-muted">{t("settings.opensky.windows")}</dt>
          <dd className="text-sf-text">
            {t("settings.opensky.windows_value", {
              done: String(st.windowsDone),
              pending: String(st.windowsPending),
            })}
          </dd>
          <dt className="text-sf-muted">{t("settings.opensky.flights")}</dt>
          <dd className="text-sf-text">
            {t("settings.opensky.flights_value", {
              day: st.flightsLastDay.toLocaleString(),
              date: st.lastDay ?? "—",
              total: st.flightsTotal.toLocaleString(),
            })}
          </dd>
          <dt className="text-sf-muted">{t("settings.opensky.credits")}</dt>
          <dd className="text-sf-text">
            {t("settings.opensky.credits_value", {
              used: String(st.creditsToday),
              budget: String(st.dailyBudget),
              remaining: st.remaining != null ? st.remaining.toLocaleString() : "—",
            })}
          </dd>
          <dt className="text-sf-muted">{t("settings.opensky.aircraft_db")}</dt>
          <dd className="text-sf-text">
            {st.aircraftDbRows > 0
              ? t("settings.opensky.aircraft_db_value", {
                  rows: st.aircraftDbRows.toLocaleString(),
                  version: st.aircraftDbVersion
                    ? new Date(st.aircraftDbVersion).toLocaleDateString([], { month: "short", year: "numeric" })
                    : "—",
                  adsbdb: String(st.adsbdbToday),
                })
              : t("settings.opensky.aircraft_db_pending")}
          </dd>
          {st.airlines.length > 0 && (
            <>
              <dt className="text-sf-muted">
                {t("settings.opensky.airlines", { date: st.lastDay ?? "—" })}
              </dt>
              <dd className="font-mono text-sf-text">
                {st.airlines.map((a) => `${a.icao} ${a.flights}`).join(" · ")}
              </dd>
            </>
          )}
        </dl>
      )}
      {st?.lastError && (
        <p className="mt-2 text-[11px] text-amber-300">
          {t("settings.opensky.last_error", { error: st.lastError })}
        </p>
      )}
      <p className="mt-2 text-[10px] text-sf-muted">{t("settings.opensky.attribution")}</p>
    </div>
  );
}

function stopLabel(reason: string): string {
  switch (reason) {
    case "daily_budget":
      return t("settings.opensky.stop.budget");
    case "rate_limited":
      return t("settings.opensky.stop.rate");
    case "no_airlines":
      return t("settings.opensky.stop.no_airlines");
    default:
      return reason;
  }
}
