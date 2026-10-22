import { useEffect, useMemo, useState } from "react";
import { Loader2, Plane, Search, User } from "lucide-react";
import { api } from "../lib/tauri";
import { t, getActiveLocale } from "../lib/i18n";
import type { AirlineFleet } from "../lib/types";

/** Filas que se pintan antes de "Mostrar todos". */
const PAGE = 120;

/** (v8.3 · Etapa 6.2b) Flota real de una aerolínea: los aviones que OpenSky
 *  la vio volar en 30 días (matrícula y tipo de la base de aviones) más los
 *  aviones del usuario con esa aerolínea, marcados como suyos. */
export function FleetPanel({ icao }: { icao: string | null }) {
  const [fleet, setFleet] = useState<AirlineFleet | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [all, setAll] = useState(false);

  useEffect(() => {
    if (!icao) return;
    let alive = true;
    setFleet(null);
    setError(null);
    api
      .airlineFleet(icao)
      .then((f) => alive && setFleet(f))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [icao]);

  const rows = useMemo(() => {
    const list = fleet?.aircraft ?? [];
    const term = q.trim().toUpperCase();
    if (!term) return list;
    return list.filter((a) =>
      [a.registration, a.typecode, a.model, a.lastAirport, a.icao24]
        .some((v) => (v ?? "").toUpperCase().includes(term)),
    );
  }, [fleet, q]);

  if (!icao) {
    return (
      <div className="rounded-2xl border border-sf-border bg-sf-surface p-6 text-center text-sm text-sf-muted">
        {t("economy.fleet.no_icao")}
      </div>
    );
  }
  if (error) {
    return (
      <div className="rounded-2xl border border-red-500/30 bg-red-500/10 p-4 text-sm text-red-300">
        {t("economy.fleet.error")}: {error}
      </div>
    );
  }
  if (!fleet) {
    return (
      <div className="flex items-center justify-center gap-2 rounded-2xl border border-sf-border bg-sf-surface p-8 text-sm text-sf-muted">
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("economy.fleet.loading")}
      </div>
    );
  }

  const locale = getActiveLocale() === "es" ? "es-ES" : "en-US";
  const fmtDay = (ts: number | null) =>
    ts ? new Date(ts * 1000).toLocaleDateString(locale, { day: "2-digit", month: "short" }) : "—";
  const shown = all ? rows : rows.slice(0, PAGE);

  return (
    <div className="space-y-3">
      <div className="rounded-2xl border border-sf-border bg-sf-surface p-4">
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
          <h3 className="flex items-center gap-1.5 text-sm font-semibold text-sf-text">
            <Plane className="h-4 w-4 text-brand-400" />
            {t("economy.fleet.title")}
          </h3>
          <span className="text-xs text-sf-muted">
            {t("economy.fleet.summary", {
              seen: String(fleet.seen),
              mine: String(fleet.mine),
            })}
          </span>
          {fleet.registeredNotSeen > 0 && (
            <span className="text-xs text-sf-muted/80">
              {t("economy.fleet.not_seen", { n: String(fleet.registeredNotSeen) })}
            </span>
          )}
        </div>
        <p className="mt-1 text-[11px] text-sf-muted">{t("economy.fleet.hint")}</p>
        {!fleet.dbReady && (
          <p className="mt-1 text-[11px] text-amber-300">{t("economy.fleet.db_loading")}</p>
        )}
        {fleet.unknown > 0 && (
          <p className="mt-1 text-[11px] text-sf-muted">
            {t("economy.fleet.unknown", { n: String(fleet.unknown) })}
          </p>
        )}
      </div>

      {fleet.aircraft.length === 0 ? (
        <div className="rounded-2xl border border-sf-border bg-sf-surface p-6 text-center text-sm text-sf-muted">
          {t("economy.fleet.empty")}
        </div>
      ) : (
        <div className="overflow-hidden rounded-2xl border border-sf-border bg-sf-surface">
          <div className="flex items-center gap-2 border-b border-sf-border px-3 py-2">
            <Search className="h-3.5 w-3.5 text-sf-muted" />
            <input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder={t("economy.fleet.search")}
              className="w-full bg-transparent text-xs text-sf-text placeholder:text-sf-muted/70 focus:outline-none"
            />
            <span className="shrink-0 text-[11px] text-sf-muted">{rows.length}</span>
          </div>
          <div className="max-h-[60vh] overflow-y-auto">
            <table className="w-full text-xs">
              <thead className="sticky top-0 bg-sf-surface text-[11px] uppercase tracking-wide text-sf-muted">
                <tr>
                  <th className="px-3 py-2 text-left font-medium">{t("economy.fleet.col.reg")}</th>
                  <th className="px-3 py-2 text-left font-medium">{t("economy.fleet.col.type")}</th>
                  <th className="px-3 py-2 text-right font-medium">{t("economy.fleet.col.flights")}</th>
                  <th className="px-3 py-2 text-left font-medium">{t("economy.fleet.col.last")}</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((a, i) => (
                  <tr
                    key={a.icao24 ?? `mine-${a.registration}-${i}`}
                    className={`border-t border-sf-border/60 ${a.mine ? "bg-brand-500/10" : ""}`}
                  >
                    <td className="px-3 py-1.5">
                      <span className="font-mono font-semibold text-sf-text">
                        {a.registration ?? a.icao24?.toUpperCase() ?? "—"}
                      </span>
                      {a.mine && (
                        <span className="ml-2 inline-flex items-center gap-0.5 rounded-full bg-brand-500/20 px-1.5 py-0.5 text-[10px] font-semibold text-brand-200">
                          <User className="h-2.5 w-2.5" />
                          {t("economy.fleet.mine")}
                        </span>
                      )}
                    </td>
                    <td className="px-3 py-1.5 text-sf-text/90">
                      <span className="font-mono">{a.typecode ?? ""}</span>
                      {a.model && (
                        <span className="ml-1.5 text-sf-muted">{a.model}</span>
                      )}
                    </td>
                    <td className="px-3 py-1.5 text-right font-mono text-sf-text">{a.flights30d}</td>
                    <td className="px-3 py-1.5 text-sf-muted">
                      {a.lastSeen ? (
                        <>
                          <span className="font-mono text-sf-text/90">{a.lastAirport ?? "?"}</span>
                          <span className="ml-1.5">{fmtDay(a.lastSeen)}</span>
                        </>
                      ) : (
                        t("economy.fleet.never_seen")
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {!all && rows.length > PAGE && (
            <button
              onClick={() => setAll(true)}
              className="w-full border-t border-sf-border py-2 text-xs font-medium text-sf-muted hover:bg-sf-raised hover:text-sf-text"
            >
              {t("economy.fleet.show_all", { n: String(rows.length) })}
            </button>
          )}
        </div>
      )}
      <p className="text-[10px] text-sf-muted">{t("economy.fleet.attribution")}</p>
    </div>
  );
}
