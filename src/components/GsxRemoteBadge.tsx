import { useEffect, useState } from "react";
import { Truck } from "lucide-react";
import { api } from "../lib/tauri";
import { useFlightLogStore } from "../stores/useFlightLogStore";

interface GsxRemote {
  connected: boolean;
  host: string | null;
  status: string | null;
  percent: number | null;
  raw: string | null;
}

/**
 * (v6.2.38) Badge de estado remoto de GSX. GSX Pro expone su estado en la
 * LAN (puerto 8744, IP que rota); lo descubrimos por sondeo. Muestra la
 * fase (boarding/deboarding/…) + porcentaje arriba. Sólo sondea mientras
 * el sim corre, para no escanear la subred en vano.
 */
export function GsxRemoteBadge() {
  const simRunning = useFlightLogStore((s) => s.status?.simRunning ?? false);
  const [st, setSt] = useState<GsxRemote | null>(null);

  useEffect(() => {
    if (!simRunning) {
      setSt(null);
      return;
    }
    let alive = true;
    const tick = () => {
      api
        .gsxRemoteStatus()
        .then((s) => {
          if (alive) setSt(s as GsxRemote);
        })
        .catch(() => {});
    };
    tick();
    const id = window.setInterval(tick, 10000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [simRunning]);

  if (!st?.connected || !st.status) return null;

  return (
    <span
      title={st.host ? `GSX @ ${st.host}` : "GSX"}
      className="inline-flex items-center gap-1.5 rounded-full border border-cyan-500/30 bg-cyan-500/10 px-2.5 py-1 text-[11px] font-medium text-cyan-200"
    >
      <Truck className="h-3 w-3" />
      <span className="uppercase tracking-wide">{st.status}</span>
      {st.percent != null && (
        <span className="font-semibold text-cyan-100">{st.percent}%</span>
      )}
    </span>
  );
}
