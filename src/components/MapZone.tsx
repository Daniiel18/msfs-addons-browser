import { useEffect, useState, type PointerEvent } from "react";
import type { Map as MapLibreMap } from "maplibre-gl";
import { BoxSelect } from "lucide-react";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import type { CommunityPackage } from "../lib/types";
import { useDispatchStore } from "../stores/useDispatchStore";
import { useToastStore } from "../stores/useToastStore";
import { DetailModal } from "./DetailModal";

interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/** Menos que esto es un clic, no un rectángulo. */
const MIN_BOX_PX = 8;

/**
 * (06/11) Dibuja un rectángulo sobre el mapa. Se pregunta al mapa por lo que ha
 * pintado dentro: en pantalla el antimeridiano no existe, y los escenarios del
 * simulador están en otra capa, así que nunca entran.
 */
export function MapZoneOverlay({
  map,
  onPick,
  onCancel,
}: {
  map: MapLibreMap;
  onPick: (folders: string[]) => void;
  onCancel: () => void;
}) {
  const [box, setBox] = useState<Box | null>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onCancel();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onCancel]);

  const at = (e: PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  };
  const finish = () => {
    if (!box) return;
    setBox(null);
    const [left, right] = [Math.min(box.x0, box.x1), Math.max(box.x0, box.x1)];
    const [top, bottom] = [Math.min(box.y0, box.y1), Math.max(box.y0, box.y1)];
    if (right - left < MIN_BOX_PX || bottom - top < MIN_BOX_PX) return;
    const found = map.queryRenderedFeatures(
      [
        [left, top],
        [right, bottom],
      ],
      { layers: ["package-point"] },
    );
    onPick([...new Set(found.map((f) => String(f.properties?.folderName)))]);
  };

  return (
    <div
      className="absolute inset-0 cursor-crosshair touch-none"
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        const p = at(e);
        setBox({ x0: p.x, y0: p.y, x1: p.x, y1: p.y });
      }}
      onPointerMove={(e) => {
        const p = at(e);
        setBox((b) => (b ? { ...b, x1: p.x, y1: p.y } : b));
      }}
      onPointerUp={finish}
    >
      {box && (
        <div
          className="pointer-events-none absolute border-2 border-brand-400 bg-brand-400/15"
          style={{
            left: Math.min(box.x0, box.x1),
            top: Math.min(box.y0, box.y1),
            width: Math.abs(box.x1 - box.x0),
            height: Math.abs(box.y1 - box.y0),
          }}
        />
      )}
      <p className="pointer-events-none absolute bottom-3 left-1/2 -translate-x-1/2 rounded-md bg-slate-950/85 px-3 py-1.5 text-xs text-white">
        {t("map.zone.hint")}
      </p>
    </div>
  );
}

/** (06/11) Lo que hay dentro de la zona, antes de tocar nada. */
export function MapZoneModal({ packages, onClose }: { packages: CommunityPackage[]; onClose: () => void }) {
  const keep = useDispatchStore((s) => s.keep);
  const [busy, setBusy] = useState(false);
  const kept = new Set(keep.map((f) => f.toLowerCase()));
  const lit = packages.filter((p) => p.enabled !== false);
  const dark = packages.filter((p) => p.enabled === false);
  const canOff = lit.filter((p) => !kept.has(p.folderName.toLowerCase()));
  const names = (list: CommunityPackage[]) => list.map((p) => p.folderName);

  const apply = async (on: CommunityPackage[], off: CommunityPackage[]) => {
    setBusy(true);
    const toast = useToastStore.getState();
    try {
      await api.dispatchApply({
        on: names(on),
        off: names(off),
        restoreName: off.length > 0 ? t("map.zone.restore_name", { n: off.length }) : null,
        markPending: off.length > 0,
        clearPending: off.length > 0,
      });
      toast.push({
        kind: "success",
        title: off.length > 0 ? t("map.zone.done_off", { n: off.length }) : t("map.zone.done_on", { n: on.length }),
        ttlMs: 2600,
      });
      onClose();
    } catch (e) {
      toast.push({ kind: "error", title: t("map.zone.error"), message: String(e) });
      setBusy(false);
    }
  };

  return (
    <DetailModal
      title={t("map.zone.title", { n: packages.length })}
      subtitle={t("map.zone.subtitle", { on: lit.length, off: dark.length })}
      icon={<BoxSelect className="h-4 w-4" />}
      onClose={onClose}
      zClass="z-[64]"
      footer={
        <div className="flex w-full items-center justify-end gap-2">
          <button
            onClick={onClose}
            className="rounded-lg border border-sf-border px-3 py-1.5 text-xs font-medium text-sf-text hover:bg-sf-raised"
          >
            {t("dispatch.review.cancel")}
          </button>
          <button
            onClick={() => void apply([], canOff)}
            disabled={busy || canOff.length === 0}
            className="rounded-lg border border-sf-border bg-sf-raised px-3 py-1.5 text-xs font-semibold text-sf-text hover:bg-sf-surface disabled:opacity-50"
          >
            {t("map.zone.turn_off", { n: canOff.length })}
          </button>
          <button
            onClick={() => void apply(dark, [])}
            disabled={busy || dark.length === 0}
            className="rounded-lg bg-brand-500 px-3 py-1.5 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:opacity-50"
          >
            {t("map.zone.turn_on", { n: dark.length })}
          </button>
        </div>
      }
    >
      <div className="space-y-2">
        <ul className="divide-y divide-sf-border overflow-hidden rounded-xl border border-sf-border bg-sf-surface">
          {packages.map((p) => (
            <li key={p.folderName} className="flex items-center gap-2 px-3 py-1.5 text-xs">
              <span className="w-12 shrink-0 font-mono font-semibold text-brand-300">{p.icao}</span>
              <span className="min-w-0 flex-1 truncate text-sf-text" title={p.folderName}>
                {p.airportName ?? p.title}
              </span>
              <span className={p.enabled === false ? "shrink-0 text-sf-muted" : "shrink-0 text-emerald-300"}>
                {t(p.enabled === false ? "map.zone.state_off" : "map.zone.state_on")}
              </span>
            </li>
          ))}
        </ul>
        {lit.length > canOff.length && (
          <p className="text-[11px] leading-snug text-sf-muted">{t("map.zone.kept", { n: lit.length - canOff.length })}</p>
        )}
        <p className="text-[11px] leading-snug text-sf-muted">{t("map.zone.restore_hint")}</p>
      </div>
    </DetailModal>
  );
}
