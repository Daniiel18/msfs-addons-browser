import { useEffect, useState } from "react";
import { Film, ChevronDown, FolderOpen, CheckCircle2 } from "lucide-react";
import type { RecordingConfig, EngineStatus } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * (v6 #2b) Ajustes de "Best Landings" (grabación). Motor: windows-record
 * (audio del sistema nativo + captura DX/fullscreen + replay buffer),
 * integrado en la app — sin descargas. Graba la ventana de MSFS directamente,
 * así que no hay selector de monitor: opciones = posición OSD, duración,
 * micrófono y carpeta.
 */
export function RecordingSettings() {
  const [open, setOpen] = useState(false);
  const [cfg, setCfg] = useState<RecordingConfig | null>(null);
  const [engine, setEngine] = useState<EngineStatus | null>(null);

  useEffect(() => {
    api.recordingConfig().then(setCfg).catch(() => {});
    api.recordingEngineStatus().then(setEngine).catch(() => {});
  }, []);

  const setKey = async (key: string, value: string) => {
    await api.setAppSetting(key, value).catch(() => {});
  };
  const patch = (p: Partial<RecordingConfig>) =>
    setCfg((c) => (c ? { ...c, ...p } : c));

  if (!cfg) return null;

  const pickFolder = async () => {
    const folder = await api.pickFolderPath().catch(() => null);
    if (folder) {
      patch({ outputPath: folder });
      await setKey("rec_output_path", folder);
    }
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
          <Film className="h-4 w-4 text-brand-400" />
          <span>
            <span className="text-xs text-slate-200">{t("rec.title")}</span>
            <span className="mt-0.5 block text-[11px] text-slate-500">
              {t("rec.subtitle")}
            </span>
          </span>
        </span>
        <ChevronDown
          className={`h-4 w-4 text-slate-500 transition-transform ${open ? "rotate-180" : ""}`}
        />
      </button>

      {open && (
        <div className="mt-3 space-y-3">
          <RecToggle
            label={t("rec.enabled")}
            checked={cfg.enabled}
            onChange={(next) => {
              patch({ enabled: next });
              void setKey("rec_enabled", next ? "1" : "0");
            }}
          />

          {/* OSD del FPM al aterrizar */}
          <RecToggle
            label={t("rec.osd_enabled")}
            checked={cfg.osdEnabled}
            onChange={(next) => {
              patch({ osdEnabled: next });
              void setKey("rec_osd_enabled", next ? "1" : "0");
            }}
          />
          <button
            onClick={() => api.osdTest().catch(() => {})}
            className="self-start rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1 text-[11px] text-slate-300 hover:border-brand-500/40"
          >
            {t("rec.osd_test")}
          </button>

          {/* Posición OSD: Arriba / Abajo */}
          <Field label={t("rec.position")}>
            <Segmented
              options={[
                { value: 0, label: t("rec.pos.top") },
                { value: 1, label: t("rec.pos.bottom") },
              ]}
              value={cfg.osdPosition}
              onChange={(v) => {
                patch({ osdPosition: v });
                void setKey("rec_osd_position", String(v));
              }}
            />
          </Field>

          {/* Calidad / rendimiento: FPS.
              (v6.2.16) Se retiró la opción de 60fps: el encoder nativo acumula
              memoria sin control a 60 (hasta 20 GB → crashea el sim) y a 30 se
              mantiene estable. El backend además fuerza el tope aunque el
              setting viejo dijera 60. */}
          <Field label={t("rec.fps")}>
            <Segmented
              options={[
                { value: 24, label: "24 fps" },
                { value: 30, label: t("rec.fps_30") },
              ]}
              value={cfg.fps <= 24 ? 24 : 30}
              onChange={(v) => {
                patch({ fps: v });
                void setKey("rec_fps", String(v));
              }}
            />
            <p className="mt-1 text-[10px] leading-relaxed text-amber-500/80">
              {t("rec.fps_60_removed")}
            </p>
          </Field>

          {/* Duración + Ilimitado */}
          <Field label={t("rec.duration")}>
            <div className="flex items-center gap-2">
              <input
                type="number"
                min={5}
                max={600}
                disabled={cfg.unlimited}
                value={cfg.clipSeconds}
                onChange={(e) => patch({ clipSeconds: Number(e.target.value) })}
                onBlur={() => {
                  const n = Math.min(600, Math.max(5, cfg.clipSeconds || 45));
                  patch({ clipSeconds: n });
                  void setKey("rec_clip_seconds", String(n));
                }}
                className="w-20 rounded-md border border-slate-800 bg-slate-950/40 px-2 py-1.5 text-xs text-slate-200 focus:border-brand-500/50 focus:outline-none disabled:opacity-40"
              />
              <button
                type="button"
                onClick={() => {
                  const next = !cfg.unlimited;
                  patch({ unlimited: next });
                  void setKey("rec_unlimited", next ? "1" : "0");
                }}
                className={`rounded-md border px-2.5 py-1.5 text-[11px] ${
                  cfg.unlimited
                    ? "border-brand-500/50 bg-brand-500/15 text-brand-200"
                    : "border-slate-800 bg-slate-900/60 text-slate-400"
                }`}
              >
                {t("rec.unlimited")}
              </button>
            </div>
          </Field>

          {/* Carpeta */}
          <Field label={t("rec.folder")}>
            <div className="flex items-center gap-2">
              <div className="min-w-0 flex-1 truncate rounded-md border border-slate-800 bg-slate-950/40 px-2 py-1.5 font-mono text-[11px] text-slate-300">
                {cfg.outputPath}
              </div>
              <button
                onClick={pickFolder}
                className="rounded-md border border-slate-800 bg-slate-900/60 px-2 py-1.5 text-[11px] text-slate-300 hover:border-brand-500/40"
              >
                {t("rec.change")}
              </button>
              <button
                onClick={() => api.openLocalPath(cfg.outputPath).catch(() => {})}
                title={t("rec.open_folder")}
                className="rounded-md border border-slate-800 bg-slate-900/60 p-1.5 text-slate-300 hover:border-brand-500/40"
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </button>
            </div>
          </Field>

          {/* Estado del motor */}
          <div className="flex items-center gap-1.5 rounded-lg border border-slate-800 bg-slate-950/40 px-2.5 py-2 text-[11px]">
            {engine?.available ? (
              <>
                <CheckCircle2 className="h-3.5 w-3.5 text-brand-400" />
                <span className="text-slate-400">
                  {t("rec.engine_ok", { engine: engine.engine })}
                </span>
              </>
            ) : (
              <span className="text-amber-300">{t("rec.engine_unavailable")}</span>
            )}
          </div>

          <p className="text-[11px] leading-relaxed text-slate-600">
            {t("rec.note")}
          </p>
        </div>
      )}
    </div>
  );
}

function RecToggle({
  label,
  icon,
  checked,
  onChange,
}: {
  label: string;
  icon?: React.ReactNode;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="flex items-center gap-1.5 text-xs text-slate-300">
        {icon}
        {label}
      </span>
      <button
        type="button"
        onClick={() => onChange(!checked)}
        className={`relative h-6 w-11 shrink-0 rounded-full border p-0 transition-colors ${
          checked
            ? "border-brand-400 bg-brand-500"
            : "border-slate-600 bg-slate-700"
        }`}
      >
        <span
          className={`absolute left-0 top-0.5 h-5 w-5 rounded-full bg-white shadow-md transition-transform ${
            checked ? "translate-x-[22px]" : "translate-x-0.5"
          }`}
        />
      </button>
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="mb-1 text-[11px] text-slate-500">{label}</div>
      {children}
    </div>
  );
}

function Segmented({
  options,
  value,
  onChange,
}: {
  options: { value: number; label: string }[];
  value: number;
  onChange: (v: number) => void;
}) {
  return (
    <div className="grid auto-cols-fr grid-flow-col gap-1">
      {options.map((o) => (
        <button
          key={o.value}
          onClick={() => onChange(o.value)}
          className={`rounded-md border px-2 py-1.5 text-[11px] ${
            value === o.value
              ? "border-brand-500/50 bg-brand-500/15 text-brand-200"
              : "border-slate-800 bg-slate-900/60 text-slate-400 hover:text-slate-200"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
