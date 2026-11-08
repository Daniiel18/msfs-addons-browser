import { useEffect, useRef, useState } from "react";
import { Armchair, ClipboardList, Loader2, PlaneTakeoff, SlidersHorizontal } from "lucide-react";
import type { TechLogAircraft } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";
import { useToastStore } from "../stores/useToastStore";
import type { HangarFocusTarget, TechLogSubTab } from "../stores/useAppStore";
import { actionError, TechLogPanel } from "./TechLogPanel";
import { TechLogCockpit } from "./TechLogCockpit";
import { TechLogCabin } from "./TechLogCabin";
import { TechLogFlights } from "./TechLogFlights";

/**
 * (01/11) Etapa 5a — Mantenimiento de un avión con Fenix/PMDG TechLog en
 * sub-pestañas, como el riel de TechLog: Estado (el panel de siempre) ·
 * Cockpit · Cabina · Vuelos. Al entrar en las tres nuevas con TechLog
 * cerrado, SimFleet lo abre minimizado (decisión del usuario).
 */

const SUBS: { key: TechLogSubTab; icon: typeof ClipboardList }[] = [
  { key: "status", icon: ClipboardList },
  { key: "cockpit", icon: SlidersHorizontal },
  { key: "cabin", icon: Armchair },
  { key: "flights", icon: PlaneTakeoff },
];

/** Fuentes que SimFleet ya intentó abrir en esta sesión (no insistir). */
const launchTried = new Set<string>();
/** Última petición atendida: al volver a la pestaña no se repite. */
let lastRequest = 0;

export function TechLogTabs({
  data,
  onReload,
  loading,
  model,
  request,
}: {
  data: TechLogAircraft;
  onReload: () => void;
  loading: boolean;
  model?: string | null;
  /** Petición del FlightBook / avisos: sub-pestaña y vuelo a abrir. */
  request?: (HangarFocusTarget & { n: number }) | null;
}) {
  const pushToast = useToastStore((s) => s.push);
  const [sub, setSub] = useState<TechLogSubTab>("status");
  const [focusFlight, setFocusFlight] = useState<{ id: number; n: number } | null>(null);
  useEffect(() => {
    if (!request || request.n <= lastRequest) return;
    lastRequest = request.n;
    setSub(request.sub);
    if (request.techlogFlightId != null) setFocusFlight({ id: request.techlogFlightId, n: request.n });
  }, [request]);

  // Abre TechLog minimizado al entrar en Cockpit, Cabina o Vuelos (una vez
  // por sesión y app: si el usuario lo cierra, no se lo volvemos a abrir).
  const [launching, setLaunching] = useState(false);
  const [launchEpoch, setLaunchEpoch] = useState(0);
  const reloadRef = useRef(onReload);
  reloadRef.current = onReload;
  useEffect(() => {
    // (02/11) El Tech Log propio no es una app aparte: nada que abrir.
    if (sub === "status" || data.source === "simfleet" || data.running || launchTried.has(data.source)) return;
    launchTried.add(data.source);
    let alive = true;
    setLaunching(true);
    api
      .techlogLaunch(data.source)
      .then(() => {
        if (!alive) return;
        reloadRef.current();
        setLaunchEpoch((n) => n + 1);
      })
      .catch((e) =>
        pushToast({ kind: "error", title: t("techlog.tabs.launch_failed", { app: data.appName }), message: actionError(String(e)) }),
      )
      .finally(() => alive && setLaunching(false));
    return () => {
      alive = false;
    };
  }, [sub, data.running, data.source, data.appName, pushToast]);

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-1 rounded-xl border border-sf-border bg-sf-bg/40 p-1">
        {SUBS.map(({ key, icon: Icon }) => (
          <button
            key={key}
            onClick={() => setSub(key)}
            className={cn(
              "inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors",
              sub === key ? "bg-brand-500/15 text-brand-200 ring-1 ring-brand-500/40" : "text-sf-muted hover:bg-sf-raised hover:text-sf-text",
            )}
          >
            <Icon className="h-3.5 w-3.5" />
            {t(`techlog.tabs.${key}`)}
          </button>
        ))}
        {launching && (
          <span className="ml-auto inline-flex items-center gap-1.5 px-2 text-[11px] text-sf-muted">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            {t("techlog.tabs.launching", { app: data.appName })}
          </span>
        )}
      </div>

      {sub === "status" && <TechLogPanel data={data} onReload={onReload} loading={loading} model={model} />}
      {sub === "cockpit" && <TechLogCockpit data={data} refreshKey={launchEpoch} />}
      {sub === "cabin" && <TechLogCabin data={data} onReload={onReload} refreshKey={launchEpoch} />}
      {sub === "flights" && <TechLogFlights data={data} refreshKey={launchEpoch} focus={focusFlight} />}
    </div>
  );
}

/** Aviso común: el avión no es el activo de su TechLog o TechLog está cerrado. */
export function TechLogPageNotice({
  state,
  appName,
  activeRegistration,
  what,
}: {
  state: string;
  appName: string;
  activeRegistration: string | null;
  /** "cockpit" | "cabin" */
  what: string;
}) {
  const text =
    state === "not_active"
      ? activeRegistration
        ? t(`techlog.page.not_active_other`, { app: appName, reg: activeRegistration, what: t(`techlog.page.what_${what}`) })
        : t(`techlog.page.not_active`, { app: appName, what: t(`techlog.page.what_${what}`) })
      : state === "closed"
        ? t("techlog.page.closed", { app: appName })
        : state === "unsupported"
          ? t("techlog.page.unsupported", { app: appName })
          : t("techlog.page.unmanaged");
  return (
    <p className="rounded-lg border border-sf-border bg-sf-raised/40 px-3 py-2 text-[11px] text-sf-muted">{text}</p>
  );
}
