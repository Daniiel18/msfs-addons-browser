import { useEffect, useState } from "react";
import {
  Film,
  ChevronDown,
  FolderOpen,
  CheckCircle2,
  Loader2,
  Video,
  Monitor,
  Volume2,
} from "lucide-react";
import type {
  RecordingConfig,
  FfmpegStatus,
  MonitorInfo,
} from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useToastStore } from "../stores/useToastStore";

/**
 * (v6 #2b) Ajustes de "Best Landings" (grabación) — replica las opciones de
 * LandingToast (Target/Position/Duration+Unlimit/Source/Path) SIN su
 * "auto-launch/exit con el sim" (lo maneja SimFleet). ffmpeg se auto-instala
 * (el usuario no descarga nada a mano).
 */
export function RecordingSettings() {
  const [open, setOpen] = useState(false);
  const [cfg, setCfg] = useState<RecordingConfig | null>(null);
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatus | null>(null);
  const [monitors, setMonitors] = useState<MonitorInfo[]>([]);
  const [audioDevices, setAudioDevices] = useState<string[]>([]);
  const [installing, setInstalling] = useState(false);
  const [testing, setTesting] = useState(false);
  const push = useToastStore((s) => s.push);

  const loadFfmpeg = () =>
    api.recordingFfmpegStatus().then(setFfmpeg).catch(() => {});

  useEffect(() => {
    api.recordingConfig().then(setCfg).catch(() => {});
    api.listMonitors().then(setMonitors).catch(() => {});
    loadFfmpeg();
  }, []);

  // Auto-instalar ffmpeg en segundo plano cuando la grabación está ACTIVA y
  // falta el binario (sin botón de descarga; no baja 80MB por solo abrir
  // Ajustes). En releases viene bundleado, así que normalmente no se ejecuta.
  useEffect(() => {
    if (!cfg?.enabled || !ffmpeg || ffmpeg.present || installing) return;
    setInstalling(true);
    api
      .recordingDownloadFfmpeg()
      .then((st) => {
        setFfmpeg(st);
        api.listAudioDevices().then(setAudioDevices).catch(() => {});
      })
      .catch(() => {})
      .finally(() => setInstalling(false));
  }, [cfg?.enabled, ffmpeg, installing]);

  // Cuando ffmpeg está listo, cargamos los dispositivos de audio.
  useEffect(() => {
    if (ffmpeg?.present) {
      api.listAudioDevices().then(setAudioDevices).catch(() => {});
    }
  }, [ffmpeg?.present]);

  const setKey = async (key: string, value: string) => {
    await api.setAppSetting(key, value).catch(() => {});
  };
  const patch = (p: Partial<RecordingConfig>) =>
    setCfg((c) => (c ? { ...c, ...p } : c));

  if (!cfg) return null;

  const testClip = async () => {
    setTesting(true);
    push({ kind: "info", title: t("rec.testing"), ttlMs: 10000 });
    try {
      await api.recordingTestClip(8);
      push({ kind: "success", title: t("rec.test_ok") });
    } catch (e) {
      push({ kind: "error", title: t("rec.test_err"), message: String(e), ttlMs: 9000 });
    } finally {
      setTesting(false);
    }
  };

  const pickFolder = async () => {
    const folder = await api.pickFolderPath().catch(() => null);
    if (folder) {
      patch({ outputPath: folder });
      await setKey("rec_output_path", folder);
    }
  };

  const audioValue = cfg.audioDevice ?? "auto";

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
          {/* Grabar automáticamente */}
          <RecToggle
            label={t("rec.enabled")}
            checked={cfg.enabled}
            onChange={(next) => {
              patch({ enabled: next });
              void setKey("rec_enabled", next ? "1" : "0");
            }}
          />

          {/* Target (monitor) */}
          <Field label={t("rec.target")} icon={<Monitor className="h-3.5 w-3.5" />}>
            <select
              value={cfg.monitorIndex}
              onChange={(e) => {
                const v = Number(e.target.value);
                patch({ monitorIndex: v });
                void setKey("rec_monitor_index", String(v));
              }}
              className="w-full rounded-md border border-slate-800 bg-slate-950/40 px-2 py-1.5 text-xs text-slate-200 focus:border-brand-500/50 focus:outline-none"
            >
              {monitors.length === 0 && <option value={0}>Display 1</option>}
              {monitors.map((m) => (
                <option key={m.index} value={m.index}>
                  {m.name} ({m.width}×{m.height}){m.primary ? " ★" : ""}
                </option>
              ))}
            </select>
          </Field>

          {/* Source: Pantalla / MSFS */}
          <Field label={t("rec.source")}>
            <Segmented
              options={[
                { value: 0, label: t("rec.source.screen") },
                { value: 1, label: t("rec.source.msfs") },
              ]}
              value={cfg.sourceType}
              onChange={(v) => {
                patch({ sourceType: v });
                void setKey("rec_source_type", String(v));
              }}
            />
          </Field>

          {/* Position: Arriba / Abajo (OSD) */}
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

          {/* Duración + Unlimit */}
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

          {/* Audio */}
          <Field label={t("rec.audio")} icon={<Volume2 className="h-3.5 w-3.5" />}>
            <select
              value={audioValue}
              onChange={(e) => {
                const v = e.target.value;
                patch({ audioDevice: v });
                void setKey("rec_audio_device", v);
              }}
              className="w-full rounded-md border border-slate-800 bg-slate-950/40 px-2 py-1.5 text-xs text-slate-200 focus:border-brand-500/50 focus:outline-none"
            >
              <option value="auto">{t("rec.audio.auto")}</option>
              <option value="off">{t("rec.audio.off")}</option>
              {audioDevices.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
          </Field>
          {audioDevices.length === 0 && ffmpeg?.present && (
            <p className="text-[10px] text-slate-600">{t("rec.audio.hint")}</p>
          )}

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

          {/* Estado de ffmpeg (auto) + grabar prueba */}
          <div className="flex items-center justify-between gap-2 rounded-lg border border-slate-800 bg-slate-950/40 px-2.5 py-2">
            <span className="flex items-center gap-1.5 text-[11px]">
              {installing ? (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-slate-400" />
                  <span className="text-slate-400">{t("rec.installing")}</span>
                </>
              ) : ffmpeg?.present ? (
                <>
                  <CheckCircle2 className="h-3.5 w-3.5 text-brand-400" />
                  <span className="text-slate-400">
                    {t("rec.ffmpeg_ok", { src: ffmpeg.source })}
                  </span>
                </>
              ) : (
                <span className="text-amber-300">{t("rec.ffmpeg_pending")}</span>
              )}
            </span>
            <button
              onClick={testClip}
              disabled={testing || !ffmpeg?.present}
              className="inline-flex items-center gap-1.5 rounded-md border border-slate-700 bg-slate-900/60 px-2.5 py-1 text-[11px] font-medium text-slate-200 hover:border-brand-500/40 disabled:opacity-50"
            >
              {testing ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Video className="h-3.5 w-3.5" />
              )}
              {t("rec.test")}
            </button>
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
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-xs text-slate-300">{label}</span>
      <button
        type="button"
        onClick={() => onChange(!checked)}
        className={`relative h-6 w-11 shrink-0 rounded-full border transition-colors ${
          checked
            ? "border-brand-400 bg-brand-500"
            : "border-slate-600 bg-slate-700"
        }`}
      >
        <span
          className={`absolute top-0.5 h-5 w-5 rounded-full bg-white shadow-md transition-transform ${
            checked ? "translate-x-5" : "translate-x-0.5"
          }`}
        />
      </button>
    </div>
  );
}

function Field({
  label,
  icon,
  children,
}: {
  label: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="mb-1 flex items-center gap-1 text-[11px] text-slate-500">
        {icon}
        {label}
      </div>
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
