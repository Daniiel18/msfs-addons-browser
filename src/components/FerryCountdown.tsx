import { useEffect, useRef, useState } from "react";

/**
 * (24/10) Cuenta atrás de un traslado pedido ("4:32"): los traslados llegan
 * en 5 minutos. Llama a `onArrived` una sola vez al llegar a 0.
 */
export function FerryCountdown({ to, onArrived }: { to: string; onArrived?: () => void }) {
  const target = Date.parse(to);
  const [now, setNow] = useState(() => Date.now());
  const left = Math.max(0, Math.ceil((target - now) / 1000));
  const fired = useRef(false);

  useEffect(() => {
    if (left > 0) {
      const id = window.setTimeout(() => setNow(Date.now()), 1000);
      return () => window.clearTimeout(id);
    }
    if (!fired.current) {
      fired.current = true;
      onArrived?.();
    }
  }, [now, left, onArrived]);

  const pad = (n: number) => String(n).padStart(2, "0");
  const h = Math.floor(left / 3600);
  const m = Math.floor((left % 3600) / 60);
  const s = left % 60;
  return (
    <span className="font-mono tabular-nums text-sf-text">
      {h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`}
    </span>
  );
}
