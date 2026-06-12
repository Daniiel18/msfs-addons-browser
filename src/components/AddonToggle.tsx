import { useState } from "react";
import { Loader2 } from "lucide-react";
import type { CommunityPackage } from "../lib/types";
import { useCommunityStore } from "../stores/useCommunityStore";
import { useToastStore } from "../stores/useToastStore";
import { t } from "../lib/i18n";

/**
 * (v4.25.0) Interruptor físico de addon + hook de toggle.
 *
 * Viven en su propio archivo porque los consumen DOS vistas (las
 * cards del grid de AddonsView y los nodos del Link Map) y meterlos
 * en AddonsView crearía un import circular con LinkMapView.
 */

/** Interruptor físico reactivo. Emerald = activo, slate = apagado,
 *  spinner en el knob mientras el backend renombra layout.json. */
export function ToggleSwitch({
  on,
  busy,
  onToggle,
  small,
}: {
  on: boolean;
  busy?: boolean;
  onToggle: (e: React.MouseEvent) => void;
  small?: boolean;
}) {
  const w = small ? "h-4 w-8" : "h-5 w-10";
  const knob = small ? "h-3 w-3" : "h-4 w-4";
  const shift = small ? "translate-x-4" : "translate-x-5";
  return (
    <button
      onClick={onToggle}
      disabled={busy}
      title={on ? t("addons.toggle.on_title") : t("addons.toggle.off_title")}
      className={`relative inline-flex ${w} shrink-0 items-center rounded-full transition-colors disabled:cursor-wait ${
        on ? "bg-emerald-500" : "bg-slate-700"
      }`}
    >
      <span
        className={`inline-flex ${knob} transform items-center justify-center rounded-full bg-white shadow transition-transform ${
          on ? shift : "translate-x-0.5"
        }`}
      >
        {busy && <Loader2 className="h-2.5 w-2.5 animate-spin text-slate-500" />}
      </span>
    </button>
  );
}

/** Hook del toggle por-addon: invoca al store (cascada vía Link Map
 *  incluida) y reporta cascada/errores por toast. */
export function usePackageToggle(pkg: CommunityPackage) {
  const setEnabled = useCommunityStore((s) => s.setEnabled);
  const pushToast = useToastStore((s) => s.push);
  const [busy, setBusy] = useState(false);
  const enabled = pkg.enabled !== false;

  const toggle = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (busy) return;
    setBusy(true);
    try {
      const report = await setEnabled(pkg.folderName, !enabled);
      // Si la cascada alcanzó más addons, contárselo al usuario —
      // es la gracia del Link Map y debe ser visible.
      if (report.changed.length > 1) {
        pushToast({
          kind: "info",
          title: !enabled
            ? t("addons.toggle.cascade_on", { n: report.changed.length })
            : t("addons.toggle.cascade_off", { n: report.changed.length }),
          message: report.changed.join(" · "),
        });
      }
      if (report.failed.length > 0) {
        pushToast({
          kind: "error",
          title: t("addons.toggle.error"),
          message: report.failed.map((f) => `${f.path}: ${f.error}`).join(" · "),
        });
      }
    } catch (err) {
      pushToast({
        kind: "error",
        title: t("addons.toggle.error"),
        message: String(err),
      });
    } finally {
      setBusy(false);
    }
  };

  return { enabled, busy, toggle };
}
