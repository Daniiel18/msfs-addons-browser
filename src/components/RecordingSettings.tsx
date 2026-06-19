import { useEffect, useState } from "react";
import {
  Film,
  ChevronDown,
  Download,
  FolderOpen,
  CheckCircle2,
  XCircle,
  Loader2,
  Video,
} from "lucide-react";
import type { RecordingConfig, FfmpegStatus } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { useToastStore } from "../stores/useToastStore";

/**
 * (v6 #2b) Ajustes de "Best Landings" (grabación) — hereda la config de
 * LandingToast SIN su opción "auto-launch/exit con el sim" (eso lo maneja
 * SimFleet). Permite configurar carpeta, duración, OSD y nº de clips,
 * comprobar/descargar ffmpeg y grabar un clip de PRUEBA (testeable sin sim).
 */
export function RecordingSettings() {
  const [open, setOpen] = useState(false);
  const [cfg, setCfg] = useState<RecordingConfig | null>(null);
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatus | null>(null);
  const [downloading, setDownloading] = useState(false);
  const [testing, setTesting] = useState(false);
  const push = useToastStore((s) => s.push);

  const reload = () => {
    api.recordingConfig().then(setCfg).catch(() => {});
    api.recordingFfmpegStatus().then(setFfmpeg).catch(() => {});
  };
  useEffect(() => {
    reload();
  }, []);

  const setKey = async (key: string, value: string) => {
    await api.setAppSetting(key, value).catch(() => {});
    reload();
  };

  const patch = (p: Partial<RecordingConfig>) =>
    setCfg((c) => (c ? { ...c, ...p } : c));

  if (!cfg) return null;

  const download = async () => {
    setDownloading(true);
    push({ kind: "info", title: t("rec.downloading"), ttlMs: 4000 });
    try {
      const st = await api.recordingDownloadFfmpeg();
      setFfmpeg(st);
      push({ kind: "success", title: t("rec.download_ok") });
    } catch (e) {
      push({ kind: "error", title: t("rec.download_err"), message: String(e), ttlMs: 9000 });
    } finally {
      setDownloading(false);
    }
  };

  const testClip = async () => {
    setTesting(true);
    push({ kind: "info", title: t("rec.testing"), ttlMs: 9000 });
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
          {/* Enabled */}
          <label className="flex cursor-pointer items-center justify-between gap-3">
            <span className="text-xs text-slate-300">{t("rec.enabled")}</span>
            <button
              type="button"
              onClick={() => {
                const next = !cfg.enabled;
                patch({ enabled: next });
                void setKey("rec_enabled", next ? "1" : "0");
              }}
              className={`relative h-5 w-9 shrink-0 rounded-full border transition-colors ${
                cfg.enabled
                  ? "border-brand-400 bg-brand-500"
                  : "border-slate-600 bg-slate-700"
              }`}
            >
              <span
                className={`absolute top-0.5 h-4 w-4 rounded-full bg-white transition-transform ${
                  cfg.enabled ? "translate-x-4" : "translate-x-0.5"
                }`}
              />
            </button>
          </label>

          {/* ffmpeg status */}
          <div className="rounded-lg border border-slate-800 bg-slate-950/40 p-2.5">
            <div className="flex items-center justify-between gap-2">
              <span className="flex items-center gap-1.5 text-xs">
                {ffmpeg?.present ? (
                  <CheckCircle2 className="h-4 w-4 text-brand-400" />
                ) : (
                  <XCircle className="h-4 w-4 text-rose-400" />
                )}
                <span className="text-slate-300">
                  {ffmpeg?.present
                    ? t("rec.ffmpeg_ok", { src: ffmpeg.source })
                    : t("rec.ffmpeg_missing")}
                </span>
              </span>
              {!ffmpeg?.present && (
                <button
                  onClick={download}
                  disabled={downloading}
                  className="inline-flex items-center gap-1.5 rounded-md bg-brand-500 px-2.5 py-1 text-[11px] font-medium text-white hover:bg-brand-400 disabled:opacity-60"
                >
                  {downloading ? (
                    <Loader2 className="h-3 w-3 animate-spin" />
                  ) : (
                    <Download className="h-3 w-3" />
                  )}
                  {t("rec.download")}
                </button>
              )}
            </div>
            {ffmpeg?.path && (
              <div className="mt-1 truncate font-mono text-[10px] text-slate-600">
                {ffmpeg.path}
              </div>
            )}
          </div>

          {/* Output folder */}
          <div>
            <div className="mb-1 text-[11px] text-slate-500">{t("rec.folder")}</div>
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
          </div>

          {/* Numeric + OSD */}
          <div className="grid grid-cols-2 gap-2">
            <NumField
              label={t("rec.clip_seconds")}
              value={cfg.clipSeconds}
              min={5}
              max={120}
              onCommit={(v) => {
                patch({ clipSeconds: v });
                void setKey("rec_clip_seconds", String(v));
              }}
            />
            <NumField
              label={t("rec.max_clips")}
              value={cfg.maxClips}
              min={1}
              max={200}
              onCommit={(v) => {
                patch({ maxClips: v });
                void setKey("rec_max_clips", String(v));
              }}
            />
          </div>
          <div>
            <div className="mb-1 text-[11px] text-slate-500">{t("rec.osd_position")}</div>
            <div className="grid grid-cols-4 gap-1">
              {[0, 1, 2, 3].map((p) => (
                <button
                  key={p}
                  onClick={() => {
                    patch({ osdPosition: p });
                    void setKey("rec_osd_position", String(p));
                  }}
                  className={`rounded-md border px-2 py-1.5 text-[11px] ${
                    cfg.osdPosition === p
                      ? "border-brand-500/50 bg-brand-500/15 text-brand-200"
                      : "border-slate-800 bg-slate-900/60 text-slate-400 hover:text-slate-200"
                  }`}
                >
                  {t(`rec.osd.${p}`)}
                </button>
              ))}
            </div>
          </div>

          {/* Test record */}
          <button
            onClick={testClip}
            disabled={testing || !ffmpeg?.present}
            className="inline-flex w-full items-center justify-center gap-2 rounded-md border border-slate-700 bg-slate-900/60 px-3 py-2 text-xs font-medium text-slate-200 hover:border-brand-500/40 disabled:opacity-50"
          >
            {testing ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Video className="h-4 w-4" />
            )}
            {t("rec.test")}
          </button>

          <p className="text-[11px] leading-relaxed text-slate-600">
            {t("rec.note")}
          </p>
        </div>
      )}
    </div>
  );
}

function NumField({
  label,
  value,
  min,
  max,
  onCommit,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  onCommit: (v: number) => void;
}) {
  const [v, setV] = useState(String(value));
  useEffect(() => setV(String(value)), [value]);
  return (
    <div>
      <div className="mb-1 text-[11px] text-slate-500">{label}</div>
      <input
        type="number"
        value={v}
        min={min}
        max={max}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => {
          const n = Math.min(max, Math.max(min, Number(v) || min));
          setV(String(n));
          onCommit(n);
        }}
        className="w-full rounded-md border border-slate-800 bg-slate-950/40 px-2 py-1.5 text-xs text-slate-200 focus:border-brand-500/50 focus:outline-none"
      />
    </div>
  );
}
