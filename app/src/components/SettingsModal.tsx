import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Activity,
  AlertCircle,
  Archive,
  Bell,
  CheckCircle2,
  Cloud,
  CloudOff,
  Compass,
  Database,
  Download,
  FileJson,
  FilePlus,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  HardDrive,
  Info,
  Link2,
  Loader2,
  MinusSquare,
  Plane,
  Power,
  RotateCcw,
  Settings as SettingsIcon,
  Trash2,
  Unlink,
  Upload,
  X,
} from "lucide-react";
import type { ExportFormat } from "../lib/types";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { useGsxLocalStore } from "../stores/useGsxLocalStore";
import { useFlightLogStore } from "../stores/useFlightLogStore";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";

/**
 * Modal de configuración.
 *
 * Layout: una sola columna con secciones (General, Vuelos, Mapa,
 * Carpetas, Almacenamiento, Acerca de). Toggles aplican al instante
 * — sin botón "Guardar". Errores se renderizan al inicio del modal.
 *
 * Vive globalmente en `App.tsx` para flotar sobre cualquier vista.
 */
export function SettingsModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const settings = useSettingsStore((s) => s.settings);
  const lastError = useSettingsStore((s) => s.lastError);
  const setBoolean = useSettingsStore((s) => s.setBoolean);
  const setAutostart = useSettingsStore((s) => s.setAutostart);
  const setMinimizeToTray = useSettingsStore((s) => s.setMinimizeToTray);
  const clearCaches = useSettingsStore((s) => s.clearCaches);
  const resetSettings = useSettingsStore((s) => s.resetSettings);

  const pilotId = useSimBriefStore((s) => s.pilotId);
  const setPilotId = useSimBriefStore((s) => s.setPilotId);
  const refreshSimBrief = useSimBriefStore((s) => s.refresh);

  const [pilotDraft, setPilotDraft] = useState("");
  const [savingPilot, setSavingPilot] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [backing, setBacking] = useState(false);
  const [exporting, setExporting] = useState<ExportFormat | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  // Versión real instalada — la lee Tauri desde tauri.conf.json en
  // runtime, así nunca queda hardcoded en el bundle JS.
  const [appVersion, setAppVersion] = useState<string | null>(null);
  // (v3.1.0) Modal de aviso de reinicio tras cambio de idioma.
  const [showRestartHint, setShowRestartHint] = useState(false);
  const setLanguage = useSettingsStore((s) => s.setLanguage);

  useEffect(() => {
    if (open) setPilotDraft(pilotId ?? "");
  }, [open, pilotId]);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    // Lazy-import: en modo demo (browser puro) `@tauri-apps/api/app`
    // no existe — caemos al string del package.json del frontend que
    // sí está embebido por Vite.
    import("@tauri-apps/api/app")
      .then(({ getVersion }) => getVersion())
      .then((v) => {
        if (!cancelled) setAppVersion(v);
      })
      .catch(() => {
        // Fallback a la versión bundleada por Vite. No es la versión
        // del binario, pero al menos no muestra "—".
        import("../../package.json")
          .then((pkg) => {
            if (!cancelled) setAppVersion((pkg as { version: string }).version);
          })
          .catch(() => {
            if (!cancelled) setAppVersion(null);
          });
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const onSavePilot = async () => {
    const trimmed = pilotDraft.trim();
    if (!trimmed || trimmed === pilotId) return;
    setSavingPilot(true);
    try {
      await setPilotId(trimmed);
    } finally {
      setSavingPilot(false);
    }
  };

  const onClearCaches = async () => {
    setClearing(true);
    setFeedback(null);
    try {
      const n = await clearCaches();
      setFeedback(t("settings.feedback.caches_cleared", { count: String(n) }));
    } catch (e) {
      setFeedback(`Error: ${String(e)}`);
    } finally {
      setClearing(false);
    }
  };

  const onReset = async () => {
    if (!window.confirm(t("settings.reset.confirm"))) return;
    setResetting(true);
    setFeedback(null);
    try {
      const n = await resetSettings();
      setFeedback(t("settings.feedback.reset_done", { count: String(n) }));
    } catch (e) {
      setFeedback(`${t("common.error")}: ${String(e)}`);
    } finally {
      setResetting(false);
    }
  };

  const openPath = (path: string | null) => {
    if (!path) return;
    api
      .openLocalPath(path)
      .catch((e) => setFeedback(`${t("settings.feedback.open_failed")}: ${String(e)}`));
  };

  const onBackup = async () => {
    setBacking(true);
    setFeedback(null);
    try {
      const folder = await api.pickFolderPath();
      if (!folder) {
        setBacking(false);
        return;
      }
      const r = await api.backupCommunity(folder);
      setFeedback(
        t("settings.feedback.backup_done", {
          path: r.outputPath,
          count: String(r.packageCount),
          mb: (r.totalBytes / 1_000_000).toFixed(0),
          secs: (r.elapsedMs / 1000).toFixed(1),
        }),
      );
    } catch (e) {
      setFeedback(`${t("settings.feedback.backup_error")}: ${String(e)}`);
    } finally {
      setBacking(false);
    }
  };

  const onExport = async (format: ExportFormat) => {
    setExporting(format);
    setFeedback(null);
    try {
      const ts = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
      const ext = format === "csv" ? "csv" : format === "txt" ? "txt" : "json";
      const filename = `msfs-addons-${ts}.${ext}`;
      const filterMap: Record<ExportFormat, { name: string; extensions: string[] }> = {
        csv: { name: "CSV (Excel)", extensions: ["csv"] },
        txt: { name: "Texto plano", extensions: ["txt"] },
        json: { name: "JSON", extensions: ["json"] },
      };
      const dest = await api.pickSavePath(filename, [filterMap[format]]);
      if (!dest) {
        setExporting(null);
        return;
      }
      const r = await api.exportAddons(dest, format);
      setFeedback(
        t("settings.feedback.export_done", {
          count: String(r.rowCount),
          path: r.outputPath,
        }),
      );
    } catch (e) {
      setFeedback(`${t("settings.feedback.export_error")}: ${String(e)}`);
    } finally {
      setExporting(null);
    }
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-slate-950/70 px-4 py-12 backdrop-blur-sm"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={onClose}
        >
          <motion.div
            className="relative w-full max-w-2xl rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl"
            initial={{ y: 20, opacity: 0 }}
            animate={{ y: 0, opacity: 1 }}
            exit={{ y: 20, opacity: 0 }}
            transition={{ duration: 0.18 }}
            onClick={(e) => e.stopPropagation()}
          >
            <header className="flex items-center justify-between border-b border-slate-800 px-5 py-3">
              <div className="flex items-center gap-2">
                <SettingsIcon className="h-4 w-4 text-brand-300" />
                <h2 className="text-sm font-semibold text-slate-100">
                  {t("settings.title")}
                </h2>
              </div>
              <button
                onClick={onClose}
                className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
              >
                <X className="h-4 w-4" />
              </button>
            </header>

            <div className="space-y-5 p-5">
              {(lastError || feedback) && (
                <div
                  className={`flex items-start gap-2 rounded-md border px-3 py-2 text-xs ${
                    lastError
                      ? "border-rose-500/30 bg-rose-500/10 text-rose-200"
                      : "border-emerald-500/30 bg-emerald-500/10 text-emerald-200"
                  }`}
                >
                  <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span>{lastError ?? feedback}</span>
                </div>
              )}

              <Section title={t("settings.section.general")} icon={<Power className="h-3.5 w-3.5" />}>
                <ThemeRow
                  current={settings.theme}
                  onChange={(t) => void api.setAppSetting("pref_theme", t).then(() => useSettingsStore.getState().bootstrap())}
                />
                <LanguageRow
                  current={settings.language}
                  onChange={(l) => void setLanguage(l)}
                  onRequestRestart={() => setShowRestartHint(true)}
                />
                <Toggle
                  label={t("settings.autostart.title")}
                  hint={t("settings.autostart.hint")}
                  checked={settings.autostartEnabled}
                  onChange={(v) => setAutostart(v)}
                />
                <Toggle
                  label={t("settings.updates.title")}
                  hint={t("settings.updates.hint")}
                  checked={settings.checkUpdatesOnStart}
                  onChange={(v) => setBoolean("checkUpdatesOnStart", v)}
                />
                <Toggle
                  label={t("settings.tray.title")}
                  hint={t("settings.tray.hint")}
                  checked={settings.minimizeToTray}
                  onChange={(v) => setMinimizeToTray(v)}
                />
              </Section>

              <Section title={t("settings.section.flights")} icon={<Plane className="h-3.5 w-3.5" />}>
                <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
                  <div className="text-xs text-slate-200">{t("settings.simbrief.pilot_id_title")}</div>
                  <p className="mt-0.5 text-[11px] text-slate-500">
                    {t("settings.simbrief.pilot_id_hint")}
                  </p>
                  <div className="mt-2 flex items-center gap-2">
                    <input
                      type="text"
                      value={pilotDraft}
                      onChange={(e) => setPilotDraft(e.target.value)}
                      placeholder="50956"
                      className="flex-1 rounded-md border border-slate-800 bg-slate-950/80 px-2 py-1.5 text-xs text-slate-100 focus:border-brand-500/40 focus:outline-none focus:ring-1 focus:ring-brand-500/30"
                    />
                    <button
                      onClick={onSavePilot}
                      disabled={savingPilot || pilotDraft.trim() === (pilotId ?? "")}
                      className="rounded-md bg-brand-500/80 px-3 py-1.5 text-xs font-medium text-slate-100 hover:bg-brand-500 disabled:opacity-40"
                    >
                      {savingPilot ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : t("common.save")}
                    </button>
                    <button
                      onClick={() => refreshSimBrief()}
                      disabled={!pilotId || savingPilot}
                      title={t("settings.simbrief.refresh_tooltip")}
                      className="rounded-md border border-slate-800 bg-slate-900/60 px-3 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 disabled:opacity-40"
                    >
                      {t("common.refresh")}
                    </button>
                  </div>
                </div>
                <div className="mt-2 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2 text-[11px] text-slate-400">
                  <Activity className="mr-1 inline h-3 w-3 text-emerald-300" />
                  {t("settings.simbrief.simconnect_hint")}
                </div>
              </Section>

              <Section title={t("settings.section.map_display")} icon={<Bell className="h-3.5 w-3.5" />}>
                <Toggle
                  label={t("settings.map.simbrief_lines")}
                  hint={t("settings.map.simbrief_lines.hint")}
                  checked={settings.showSimbriefLines}
                  onChange={(v) => setBoolean("showSimbriefLines", v)}
                />
                <Toggle
                  label={t("settings.map.simconnect_lines")}
                  hint={t("settings.map.simconnect_lines.hint")}
                  checked={settings.showSimconnectLines}
                  onChange={(v) => setBoolean("showSimconnectLines", v)}
                />
              </Section>

              <Section title={t("settings.section.folders")} icon={<FolderOpen className="h-3.5 w-3.5" />}>
                <PathRow
                  label={t("settings.folders.community")}
                  hint={t("settings.folders.community.hint")}
                  path={settings.communityPath}
                  onOpen={() => openPath(settings.communityPath)}
                />
                <PathRow
                  label={t("settings.folders.logs")}
                  hint={t("settings.folders.logs.hint")}
                  path={settings.logsPath}
                  onOpen={() => openPath(settings.logsPath)}
                />
              </Section>

              <Section title={t("settings.section.backup")} icon={<Archive className="h-3.5 w-3.5" />}>
                <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
                  <div className="flex items-center gap-1.5 text-xs text-slate-200">
                    <Archive className="h-3 w-3 text-slate-500" />
                    {t("settings.backup.title")}
                  </div>
                  <p className="mt-0.5 text-[11px] text-slate-500">
                    {t("settings.backup.hint")}
                  </p>
                  <div className="mt-2">
                    <button
                      onClick={onBackup}
                      disabled={backing}
                      className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 disabled:opacity-50"
                    >
                      {backing ? (
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <Archive className="h-3.5 w-3.5" />
                      )}
                      {t("settings.backup.button")}…
                    </button>
                  </div>
                </div>
              </Section>

              <Section title={t("settings.section.gsx")} icon={<CheckCircle2 className="h-3.5 w-3.5" />}>
                <GsxProfilesPanel onFeedback={setFeedback} />
              </Section>

              <Section
                title={t("settings.section.cloud")}
                icon={<Cloud className="h-3.5 w-3.5" />}
              >
                <CloudSyncPanel onFeedback={setFeedback} />
              </Section>

              <Section title={t("settings.section.msfs_logbook")} icon={<FileText className="h-3.5 w-3.5" />}>
                <MsfsLogbookRow onFeedback={setFeedback} />
              </Section>

              <Section title={t("settings.section.import")} icon={<Upload className="h-3.5 w-3.5" />}>
                <ImportInventoryRow onFeedback={setFeedback} />
              </Section>

              <Section title={t("settings.section.export")} icon={<Download className="h-3.5 w-3.5" />}>
                <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
                  <div className="text-xs text-slate-200">
                    {t("settings.export.title")}
                  </div>
                  <p className="mt-0.5 text-[11px] text-slate-500">
                    {t("settings.export.hint")}
                  </p>
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    <ExportButton
                      icon={<FileSpreadsheet className="h-3.5 w-3.5" />}
                      label="CSV (Excel)"
                      busy={exporting === "csv"}
                      onClick={() => onExport("csv")}
                    />
                    <ExportButton
                      icon={<FileText className="h-3.5 w-3.5" />}
                      label="TXT"
                      busy={exporting === "txt"}
                      onClick={() => onExport("txt")}
                    />
                    <ExportButton
                      icon={<FileJson className="h-3.5 w-3.5" />}
                      label="JSON"
                      busy={exporting === "json"}
                      onClick={() => onExport("json")}
                    />
                  </div>
                </div>
              </Section>

              <Section title={t("settings.section.storage")} icon={<HardDrive className="h-3.5 w-3.5" />}>
                <ActionRow
                  icon={<Database className="h-3 w-3 text-slate-500" />}
                  label={t("settings.storage.clear_caches.label")}
                  hint={t("settings.storage.clear_caches.hint")}
                  buttonLabel={t("settings.storage.clear_caches.button")}
                  buttonIcon={<Trash2 className="h-3.5 w-3.5" />}
                  buttonTone="rose"
                  busy={clearing}
                  onClick={onClearCaches}
                />
                <ActionRow
                  icon={<RotateCcw className="h-3 w-3 text-slate-500" />}
                  label={t("settings.storage.reset.label")}
                  hint={t("settings.storage.reset.hint")}
                  buttonLabel={t("settings.storage.reset.button")}
                  buttonIcon={<MinusSquare className="h-3.5 w-3.5" />}
                  buttonTone="amber"
                  busy={resetting}
                  onClick={onReset}
                />
              </Section>

              <Section title={t("settings.section.tour")} icon={<Compass className="h-3.5 w-3.5" />}>
                <ActionRow
                  icon={<Compass className="h-3 w-3 text-slate-500" />}
                  label={t("settings.tour.label")}
                  hint={t("settings.tour.hint")}
                  buttonLabel={t("settings.tour.button")}
                  buttonIcon={<Compass className="h-3.5 w-3.5" />}
                  buttonTone="brand"
                  busy={false}
                  onClick={async () => {
                    await useSettingsStore
                      .getState()
                      .setOnboardingCompleted(false);
                    if (typeof window !== "undefined") {
                      window.sessionStorage.removeItem(
                        "msfs-addons:onboarding-skip-session",
                      );
                    }
                    onClose();
                    // El App.tsx ya tiene el tour montado condicionalmente
                    // sobre `onboardingCompleted` — pero como el flag se
                    // chequea sólo al primer mount del App, fuerzo el
                    // dispatch de un evento para que un listener global
                    // lo abra ahora mismo.
                    window.dispatchEvent(new CustomEvent("msfs-addons:show-tour"));
                  }}
                />
              </Section>

              <Section title={t("settings.section.about")} icon={<Info className="h-3.5 w-3.5" />}>
                <div className="space-y-1 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5 text-[11px] text-slate-400">
                  <div>
                    <span className="text-slate-500">{t("settings.about.version")}:</span>{" "}
                    <span className="font-mono text-slate-200">
                      {appVersion ?? "—"}
                    </span>
                  </div>
                  <div>
                    <span className="text-slate-500">GitHub:</span>{" "}
                    {/* (v2.2.0) Fix: el <a> con target="_blank" en
                        Tauri WebView2 NO abre el navegador externo —
                        intenta abrirlo en el WebView y no pasa nada.
                        Hay que usar el opener plugin via api.openExternal. */}
                    <button
                      onClick={() =>
                        void api.openExternal(
                          "https://github.com/Daniiel18/msfs-addons-browser/releases/latest",
                        )
                      }
                      className="text-brand-300 hover:underline"
                    >
                      {t("settings.about.github_link")}
                    </button>
                  </div>
                </div>
              </Section>
            </div>
          </motion.div>
        </motion.div>
      )}
      {showRestartHint && (
        <RestartHintModal onClose={() => setShowRestartHint(false)} />
      )}
    </AnimatePresence>
  );
}

/** Sección de Settings para configurar dónde está PMDG OC. Vive
 *  acá porque tiene su propio estado local (diagnóstico) y no
 *  encajaba en el patrón compartido `Toggle/PathRow`. */
/** Selector compacto de tema dark/light. La app está optimizada
 *  para dark, así que el modo light se marca como "Experimental"
 *  (algunos componentes hardcoded a colores dark seguirán igual).
 *  El mapa de FlightBook sí cambia de basemap según el tema. */
function ThemeRow({
  current,
  onChange,
}: {
  current: string;
  onChange: (t: string) => void;
}) {
  return (
    <div className="flex items-start justify-between gap-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="text-xs text-slate-200">{t("settings.theme.title")}</div>
        <p className="mt-0.5 text-[11px] text-slate-500">
          {t("settings.theme.description")}
        </p>
      </div>
      <div className="flex shrink-0 rounded-md border border-slate-700 bg-slate-950/50 p-0.5">
        <button
          onClick={() => onChange("dark")}
          className={`rounded px-2.5 py-1 text-[11px] ${
            current === "dark"
              ? "bg-slate-700/80 text-slate-100"
              : "text-slate-400 hover:text-slate-200"
          }`}
        >
          {t("settings.theme.dark")}
        </button>
        <button
          onClick={() => onChange("light")}
          className={`rounded px-2.5 py-1 text-[11px] ${
            current === "light"
              ? "bg-slate-700/80 text-slate-100"
              : "text-slate-400 hover:text-slate-200"
          }`}
        >
          {t("settings.theme.light")}
        </button>
      </div>
    </div>
  );
}

/** (v3.1.0 / v3.1.3) Fila del selector de idioma. Dispara modal de
 *  aviso de reinicio porque buena parte de la app sigue con strings
 *  inline que no reaccionan al cambio en caliente.
 *  AHORA awaiteamos el setLanguage antes de mostrar el modal — antes
 *  era fire-and-forget y permitía que el usuario cerrase la app
 *  ANTES de que el write a DB completara, perdiendo el valor. */
export function LanguageRow({
  current,
  onChange,
  onRequestRestart,
}: {
  current: string;
  onChange: (l: "auto" | "es" | "en") => Promise<void> | void;
  onRequestRestart: () => void;
}) {
  const handle = async (next: "auto" | "es" | "en") => {
    if (next === current) return;
    try {
      // Esperamos a que el setting se persista en DB antes de
      // ofrecer el reinicio. Si falla, no mostramos el modal.
      const result = onChange(next);
      if (result instanceof Promise) {
        await result;
      }
      onRequestRestart();
    } catch (e) {
      console.warn("language change failed:", e);
    }
  };
  return (
    <div className="flex items-start justify-between gap-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="text-xs text-slate-200">{t("settings.language.title")}</div>
        <p className="mt-0.5 text-[11px] text-slate-500">
          {t("settings.language.description")}
        </p>
      </div>
      <div className="flex shrink-0 rounded-md border border-slate-700 bg-slate-950/50 p-0.5">
        <button
          onClick={() => void handle("auto")}
          className={`rounded px-2.5 py-1 text-[11px] ${
            current === "auto"
              ? "bg-slate-700/80 text-slate-100"
              : "text-slate-400 hover:text-slate-200"
          }`}
        >
          Auto
        </button>
        <button
          onClick={() => void handle("es")}
          className={`rounded px-2.5 py-1 text-[11px] ${
            current === "es"
              ? "bg-slate-700/80 text-slate-100"
              : "text-slate-400 hover:text-slate-200"
          }`}
        >
          ES
        </button>
        <button
          onClick={() => void handle("en")}
          className={`rounded px-2.5 py-1 text-[11px] ${
            current === "en"
              ? "bg-slate-700/80 text-slate-100"
              : "text-slate-400 hover:text-slate-200"
          }`}
        >
          EN
        </button>
      </div>
    </div>
  );
}

/** (v3.1.0) Modal de aviso "reinicia SimFleet" tras cambio de
 *  idioma. */
export function RestartHintModal({ onClose }: { onClose: () => void }) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/70 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="w-full max-w-sm rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-sm font-semibold text-slate-100">
          {t("restart.title")}
        </h3>
        <p className="mt-2 text-xs leading-relaxed text-slate-400">
          {t("restart.body")}
        </p>
        <div className="mt-4 flex justify-end">
          <button
            onClick={onClose}
            className="rounded-md bg-slate-700 px-3 py-1.5 text-xs font-medium text-slate-100 hover:bg-slate-600"
          >
            {t("restart.button")}
          </button>
        </div>
      </div>
    </div>
  );
}

function Section({
  title,
  icon,
  children,
}: {
  title: string;
  icon?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div>
      <h3 className="mb-2 inline-flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-400">
        {icon}
        {title}
      </h3>
      <div className="space-y-2">{children}</div>
    </div>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      className="flex w-full cursor-pointer items-center justify-between gap-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5 text-left hover:border-slate-700"
    >
      <div className="min-w-0 flex-1">
        <div className="text-xs text-slate-200">{label}</div>
        {hint && <div className="text-[11px] text-slate-500">{hint}</div>}
      </div>
      <div
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
      </div>
    </button>
  );
}

function PathRow({
  label,
  hint,
  path,
  onOpen,
}: {
  label: string;
  hint?: string;
  path: string | null;
  onOpen: () => void;
}) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="text-xs text-slate-200">{label}</div>
        {hint && <div className="text-[11px] text-slate-500">{hint}</div>}
        {path ? (
          <div className="mt-1 truncate font-mono text-[10px] text-slate-600">{path}</div>
        ) : (
          <div className="mt-1 text-[10px] italic text-slate-600">{t("settings.path.not_detected")}</div>
        )}
      </div>
      <button
        onClick={onOpen}
        disabled={!path}
        className="inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-800 px-2.5 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 disabled:cursor-not-allowed disabled:opacity-40"
      >
        <FolderOpen className="h-3.5 w-3.5" />
        {t("common.open")}
      </button>
    </div>
  );
}

function ExportButton({
  icon,
  label,
  busy,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  busy: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={busy}
      className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 disabled:opacity-50"
    >
      {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : icon}
      {label}
    </button>
  );
}

function ActionRow({
  icon,
  label,
  hint,
  buttonLabel,
  buttonIcon,
  buttonTone,
  busy,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  hint?: string;
  buttonLabel: string;
  buttonIcon: React.ReactNode;
  buttonTone: "rose" | "amber" | "brand";
  busy: boolean;
  onClick: () => void;
}) {
  const toneClass =
    buttonTone === "rose"
      ? "hover:border-rose-500/40 hover:text-rose-300"
      : buttonTone === "amber"
        ? "hover:border-amber-500/40 hover:text-amber-300"
        : "hover:border-brand-500/40 hover:text-brand-300";
  return (
    <div className="flex items-start justify-between gap-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5 text-xs text-slate-200">
          {icon}
          {label}
        </div>
        {hint && <p className="mt-0.5 text-[11px] text-slate-500">{hint}</p>}
      </div>
      <button
        onClick={onClick}
        disabled={busy}
        className={`inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-800 px-2.5 py-1.5 text-xs text-slate-300 disabled:opacity-50 ${toneClass}`}
      >
        {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : buttonIcon}
        {buttonLabel}
      </button>
    </div>
  );
}

/** (v1.1.4) Panel de perfiles GSX — contador + instalar perfil
 *  + abrir carpeta nativa de Virtuali. El usuario referenció
 *  `DragAndDropInstaller.exe` como flujo deseado; aquí se hace
 *  con file picker (.ini/.py) + copia al `%APPDATA%\Virtuali\GSX\MSFS`.
 */
function GsxProfilesPanel({
  onFeedback,
}: {
  onFeedback: (msg: string | null) => void;
}) {
  const installedIcaos = useGsxLocalStore((s) => s.installedIcaos);
  const refresh = useGsxLocalStore((s) => s.refresh);
  const [installing, setInstalling] = useState(false);

  const onPick = async () => {
    setInstalling(true);
    onFeedback(null);
    try {
      const paths = await api.pickFilePaths([
        { name: "GSX profiles", extensions: ["ini", "py", "zip", "rar"] },
      ]);
      if (paths.length === 0) {
        setInstalling(false);
        return;
      }
      let totalInstalled = 0;
      const skippedAll: string[] = [];
      for (const p of paths) {
        try {
          const report = await api.gsxInstallProfile(p);
          totalInstalled += report.installedFiles.length;
          skippedAll.push(...report.skippedFiles);
        } catch (e) {
          onFeedback(t("settings.gsx.feedback.failed", { path: p, error: String(e) }));
          setInstalling(false);
          return;
        }
      }
      await refresh();
      const skippedMsg =
        skippedAll.length > 0
          ? t("settings.gsx.feedback.skipped_suffix", { count: String(skippedAll.length) })
          : "";
      onFeedback(
        t("settings.gsx.feedback.installed", {
          count: String(totalInstalled),
          skipped: skippedMsg,
        }),
      );
    } catch (e) {
      onFeedback(`${t("common.error")}: ${String(e)}`);
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-xs text-slate-200">
            {t("settings.gsx.installed_count")}:{" "}
            <span className="font-mono text-violet-300">
              {installedIcaos.size}
            </span>
          </div>
          <p className="mt-0.5 text-[11px] text-slate-500">
            {t("settings.gsx.installed_hint")}
          </p>
        </div>
        <button
          onClick={onPick}
          disabled={installing}
          className="inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-violet-500/40 hover:text-violet-200 disabled:opacity-50"
          title={t("settings.gsx.install_tooltip")}
        >
          {installing ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <FilePlus className="h-3.5 w-3.5" />
          )}
          {t("settings.gsx.install_button")}…
        </button>
      </div>
    </div>
  );
}

/** (v2.0.0) Sincronización con Google Drive. Tres estados:
 *
 *   1. Sin credentials → muestra inputs Client ID + Client Secret.
 *   2. Con credentials, sin conectar → botón "Conectar con Google".
 *   3. Conectado → muestra email + last sync + "Sync ahora" + "Desconectar".
 *
 *  El flow de OAuth se inicia con `cloudStartOauth()` que devuelve la
 *  URL para abrir en el navegador. Suscribimos al evento
 *  `cloud://oauth-completed` para refrescar la config cuando termine.
 */
function CloudSyncPanel({
  onFeedback,
}: {
  onFeedback: (msg: string | null) => void;
}) {
  const [config, setConfig] = useState<{
    connected: boolean;
    hasCredentials: boolean;
    userEmail: string | null;
    lastSyncAt: string | null;
  } | null>(null);
  // (v3.2.0) Form de credenciales eliminado — credenciales OAuth
  // van embebidas en el binario. Sólo conservamos los flags de
  // operaciones activas.
  const [connecting, setConnecting] = useState(false);
  // (v3.5.0 F3) Estados separados para upload / download. Antes había
  // un único "syncing" que confundía: no se sabía qué dirección iba.
  const [uploading, setUploading] = useState(false);
  const [downloading, setDownloading] = useState(false);
  // (v3.5.0) Estado del botón de purga — independiente para que no
  // bloquee/se confunda con sync.
  const [purging, setPurging] = useState(false);
  // (v2.0.2) Diagnóstico paso a paso — visible al pulsar el botón.
  const [testing, setTesting] = useState(false);
  const [testReport, setTestReport] = useState<{
    overallOk: boolean;
    steps: { name: string; ok: boolean; detail: string }[];
    hint: string | null;
  } | null>(null);
  const [showHelp, setShowHelp] = useState(false);

  const refresh = async () => {
    try {
      const c = await api.cloudGetConfig();
      setConfig(c);
    } catch (e) {
      onFeedback(`${t("common.error")}: ${String(e)}`);
    }
  };

  useEffect(() => {
    void refresh();
    let unsub: (() => void) | null = null;
    api
      .onCloudOauthCompleted(async (evt) => {
        setConnecting(false);
        if (evt.ok) {
          onFeedback(
            t("settings.cloud.feedback.connected", {
              email: evt.userEmail ?? t("settings.cloud.no_email"),
            }),
          );
        } else {
          onFeedback(
            t("settings.cloud.feedback.oauth_failed", {
              error: evt.error ?? t("common.unknown"),
            }),
          );
        }
        await refresh();
      })
      .then((u) => {
        unsub = u;
      })
      .catch(() => {});
    return () => {
      if (unsub) unsub();
    };
  }, []);

  const onConnect = async () => {
    setConnecting(true);
    onFeedback(t("settings.cloud.feedback.opening_browser"));
    try {
      const start = await api.cloudStartOauth();
      // Abrimos la URL en el navegador del sistema.
      await api.openExternal(start.authUrl);
    } catch (e) {
      setConnecting(false);
      onFeedback(`${t("settings.cloud.feedback.oauth_start_error")}: ${String(e)}`);
    }
  };

  const onDisconnect = async () => {
    if (!window.confirm(t("settings.cloud.disconnect_confirm"))) return;
    try {
      await api.cloudDisconnect();
      onFeedback(t("settings.cloud.feedback.disconnected"));
      await refresh();
    } catch (e) {
      onFeedback(`${t("common.error")}: ${String(e)}`);
    }
  };

  // (v3.5.0 F3/F4) Upload — push de TODO local → cloud. Mensaje neutro
  // sin contadores: el usuario reportó que mostrar "131 vuelos, 270378
  // puntos" era demasiada info y prefería un confirm simple.
  const onUpload = async () => {
    setUploading(true);
    onFeedback(null);
    try {
      await api.cloudUploadAll();
      onFeedback(t("settings.cloud.feedback.upload_done"));
      await refresh();
    } catch (e) {
      onFeedback(`${t("settings.cloud.feedback.upload_error")}: ${String(e)}`);
    } finally {
      setUploading(false);
    }
  };

  // (v3.5.0 F3/F4) Download — pull de cloud, skip los ya locales.
  const reloadFlightLog = useFlightLogStore((s) => s.reload);
  const onDownload = async () => {
    setDownloading(true);
    onFeedback(null);
    try {
      await api.cloudDownloadMissing();
      onFeedback(t("settings.cloud.feedback.download_done"));
      await refresh();
      await reloadFlightLog();
    } catch (e) {
      onFeedback(`${t("settings.cloud.feedback.download_error")}: ${String(e)}`);
    } finally {
      setDownloading(false);
    }
  };

  const onTestConnection = async () => {
    setTesting(true);
    setTestReport(null);
    onFeedback(null);
    try {
      const r = await api.cloudTestConnection();
      setTestReport(r);
      onFeedback(null);
    } catch (e) {
      onFeedback(`${t("settings.cloud.feedback.test_error")}: ${String(e)}`);
    } finally {
      setTesting(false);
    }
  };

  // (v3.5.0) Purga del cloud — sólo accesible cuando el usuario está
  // conectado a Google (sin auth no se puede borrar nada). El confirm
  // usa los mismos textos i18n que ya existían como placeholders.
  const onPurge = async () => {
    if (!window.confirm(t("settings.cloud.purge.confirm.body"))) return;
    setPurging(true);
    onFeedback(null);
    try {
      const r = await api.cloudPurge();
      onFeedback(
        t("settings.cloud.purge.success", {
          count: String(r.deletedFlights),
        }),
      );
      await refresh();
    } catch (e) {
      onFeedback(
        t("settings.cloud.purge.error", { message: String(e) }),
      );
    } finally {
      setPurging(false);
    }
  };

  if (config === null) {
    return (
      <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5 text-[11px] text-slate-500">
        {t("settings.cloud.loading_state")}
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {/* Estado actual */}
      <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1.5 text-xs text-slate-200">
              {config.connected ? (
                <Cloud className="h-3 w-3 text-emerald-300" />
              ) : (
                <CloudOff className="h-3 w-3 text-slate-500" />
              )}
              {config.connected
                ? t("settings.cloud.status.connected_as", {
                    email: config.userEmail ?? t("settings.cloud.no_email_short"),
                  })
                : t("settings.cloud.status.not_connected")}
            </div>
            <p className="mt-0.5 text-[11px] text-slate-500">
              {config.connected
                ? t("settings.cloud.status.connected_hint", {
                    last_sync: config.lastSyncAt
                      ? new Date(config.lastSyncAt).toLocaleString()
                      : t("settings.cloud.never"),
                  })
                : t("settings.cloud.status.not_connected_hint")}
            </p>
          </div>
          <div className="flex shrink-0 flex-col gap-1.5">
            {config.connected && (
              <>
                {/* (v3.5.0 F3) Botones separados Subir / Bajar para
                    evitar la confusión del único "Sync now" anterior. */}
                <button
                  onClick={onUpload}
                  disabled={uploading || downloading}
                  className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-emerald-500/40 hover:text-emerald-200 disabled:opacity-50"
                  title={t("settings.cloud.upload.tooltip")}
                >
                  {uploading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Upload className="h-3.5 w-3.5" />
                  )}
                  {t("settings.cloud.upload")}
                </button>
                <button
                  onClick={onDownload}
                  disabled={uploading || downloading}
                  className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-sky-500/40 hover:text-sky-200 disabled:opacity-50"
                  title={t("settings.cloud.download.tooltip")}
                >
                  {downloading ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Download className="h-3.5 w-3.5" />
                  )}
                  {t("settings.cloud.download")}
                </button>
                <button
                  onClick={onTestConnection}
                  disabled={testing}
                  className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-sky-500/40 hover:text-sky-200 disabled:opacity-50"
                  title={t("settings.cloud.test_connection.tooltip")}
                >
                  {testing ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <CheckCircle2 className="h-3.5 w-3.5" />
                  )}
                  {t("settings.cloud.test_connection")}
                </button>
                <button
                  onClick={onPurge}
                  disabled={purging}
                  className="inline-flex items-center gap-1 rounded-md border border-rose-500/30 bg-rose-500/5 px-2.5 py-1.5 text-xs text-rose-300 hover:border-rose-500/60 hover:bg-rose-500/15 disabled:opacity-50"
                  title={t("settings.cloud.purge.hint")}
                >
                  {purging ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="h-3.5 w-3.5" />
                  )}
                  {t("settings.cloud.purge.button")}
                </button>
                <button
                  onClick={onDisconnect}
                  className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-rose-500/40 hover:text-rose-300"
                >
                  <Unlink className="h-3.5 w-3.5" />
                  {t("settings.cloud.disconnect")}
                </button>
              </>
            )}
            {!config.connected && config.hasCredentials && (
              <>
                <button
                  onClick={onConnect}
                  disabled={connecting}
                  className="inline-flex items-center gap-1 rounded-md border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1.5 text-xs font-medium text-emerald-200 hover:bg-emerald-500/20 disabled:opacity-50"
                >
                  {connecting ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <Link2 className="h-3.5 w-3.5" />
                  )}
                  {t("settings.cloud.connect")}
                </button>
                <button
                  onClick={onTestConnection}
                  disabled={testing}
                  className="inline-flex items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-sky-500/40 hover:text-sky-200 disabled:opacity-50"
                  title={t("settings.cloud.test_creds.tooltip")}
                >
                  {testing ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <CheckCircle2 className="h-3.5 w-3.5" />
                  )}
                  {t("settings.cloud.test")}
                </button>
              </>
            )}
          </div>
        </div>
      </div>

      {/* (v3.2.0) UI de credenciales eliminada — las claves OAuth
          están embebidas en el binario via `build.rs` desde
          `secrets.local.toml`. Sólo se requiere login con un Gmail
          autorizado (whitelist hardcoded). El usuario pidió que esta
          pestaña quede limpia. */}

      {/* (v2.0.2) Reporte de diagnóstico — visible tras pulsar Probar */}
      {testReport && (
        <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
          <div className="mb-1.5 flex items-center justify-between gap-2">
            <div
              className={`flex items-center gap-1.5 text-xs font-semibold ${
                testReport.overallOk ? "text-emerald-300" : "text-amber-300"
              }`}
            >
              {testReport.overallOk ? (
                <CheckCircle2 className="h-3.5 w-3.5" />
              ) : (
                <AlertCircle className="h-3.5 w-3.5" />
              )}
              {testReport.overallOk
                ? t("settings.cloud.diag.ok")
                : t("settings.cloud.diag.problems")}
            </div>
            <button
              onClick={() => setTestReport(null)}
              className="rounded p-1 text-slate-500 hover:text-slate-300"
              title={t("common.close")}
            >
              <X className="h-3 w-3" />
            </button>
          </div>
          <ol className="space-y-1">
            {testReport.steps.map((s, i) => (
              <li
                key={i}
                className={`flex items-start gap-2 rounded border px-2 py-1.5 text-[11px] ${
                  s.ok
                    ? "border-emerald-500/30 bg-emerald-500/5"
                    : "border-amber-500/30 bg-amber-500/10"
                }`}
              >
                <span
                  className={`mt-0.5 inline-flex h-3 w-3 shrink-0 items-center justify-center rounded-full text-[10px] font-bold ${
                    s.ok
                      ? "bg-emerald-500/30 text-emerald-100"
                      : "bg-amber-500/30 text-amber-100"
                  }`}
                >
                  {s.ok ? "✓" : "!"}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="font-medium text-slate-200">{s.name}</div>
                  <div className="mt-0.5 break-words text-slate-400">
                    {s.detail}
                  </div>
                </div>
              </li>
            ))}
          </ol>
          {testReport.hint && (
            <p
              className={`mt-2 text-[11px] ${
                testReport.overallOk ? "text-emerald-300" : "text-amber-200"
              }`}
            >
              💡 {testReport.hint}
            </p>
          )}
        </div>
      )}

      {/* (v2.0.2) Guía paso a paso — colapsable */}
      <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
        <button
          onClick={() => setShowHelp((v) => !v)}
          className="flex w-full items-center justify-between text-left text-xs text-slate-200 hover:text-slate-100"
        >
          <span className="inline-flex items-center gap-1.5">
            <Info className="h-3.5 w-3.5 text-sky-300" />
            {t("settings.cloud.help.toggle")}
          </span>
          <span className="text-slate-500">{showHelp ? "▲" : "▼"}</span>
        </button>
        {showHelp && (
          <ol className="mt-2 space-y-2 text-[11px] text-slate-400">
            <li>
              <span className="font-semibold text-slate-200">
                1. Crea un proyecto en Google Cloud
              </span>
              <p className="mt-0.5">
                Abre{" "}
                <a
                  href="#"
                  onClick={(e) => {
                    e.preventDefault();
                    void api.openExternal(
                      "https://console.cloud.google.com/projectcreate",
                    );
                  }}
                  className="text-brand-300 hover:underline"
                >
                  console.cloud.google.com/projectcreate
                </a>{" "}
                → nombre cualquiera (ej. "msfs-addons-sync") → Create.
              </p>
            </li>
            <li>
              <span className="font-semibold text-slate-200">
                2. Activa la Google Drive API en ESE proyecto
              </span>
              <p className="mt-0.5">
                Abre{" "}
                <a
                  href="#"
                  onClick={(e) => {
                    e.preventDefault();
                    void api.openExternal(
                      "https://console.cloud.google.com/apis/library/drive.googleapis.com",
                    );
                  }}
                  className="text-brand-300 hover:underline"
                >
                  apis/library/drive.googleapis.com
                </a>
                . VERIFICA que en el desplegable de arriba está
                seleccionado TU proyecto (el del paso 1). Pulsa{" "}
                <span className="font-mono">Enable</span>. Espera 30 s.
              </p>
            </li>
            <li>
              <span className="font-semibold text-slate-200">
                3. Configura el OAuth Consent Screen
              </span>
              <p className="mt-0.5">
                Abre{" "}
                <a
                  href="#"
                  onClick={(e) => {
                    e.preventDefault();
                    void api.openExternal(
                      "https://console.cloud.google.com/auth/overview",
                    );
                  }}
                  className="text-brand-300 hover:underline"
                >
                  auth/overview
                </a>
                . User Type:{" "}
                <span className="font-mono">External</span>. Pon el
                nombre de la app y tu email como soporte. En la pantalla
                de <span className="font-mono">Test Users</span>, AÑADE
                TU PROPIO EMAIL (sin esto, Google bloquea el consent en
                modo Testing).
              </p>
            </li>
            <li>
              <span className="font-semibold text-slate-200">
                4. Crea el OAuth Client tipo Desktop
              </span>
              <p className="mt-0.5">
                Abre{" "}
                <a
                  href="#"
                  onClick={(e) => {
                    e.preventDefault();
                    void api.openExternal(
                      "https://console.cloud.google.com/apis/credentials",
                    );
                  }}
                  className="text-brand-300 hover:underline"
                >
                  apis/credentials
                </a>{" "}
                → Create Credentials → OAuth Client ID → Application
                type:{" "}
                <span className="font-mono font-semibold">
                  Desktop app
                </span>{" "}
                (NO Web). Copia el{" "}
                <span className="font-mono">Client ID</span> y{" "}
                <span className="font-mono">Client Secret</span>.
              </p>
            </li>
            <li>
              <span className="font-semibold text-slate-200">
                5. Pega las credenciales en esta app
              </span>
              <p className="mt-0.5">
                Arriba en este mismo panel → "Configurar / Cambiar" →
                pega los dos valores → Guardar.
              </p>
            </li>
            <li>
              <span className="font-semibold text-slate-200">
                6. Conectar
              </span>
              <p className="mt-0.5">
                Pulsa "Conectar con Google" → se abre tu navegador →
                eliges tu cuenta → en la pantalla{" "}
                <em>"Google no verificó esta app"</em> pulsa{" "}
                <span className="font-mono">Advanced</span> →{" "}
                <span className="font-mono">Go to {`<app>`} (unsafe)</span>{" "}
                (normal porque la app está en Testing). Aprueba TODAS
                las scopes (drive.appdata + userinfo.email).
              </p>
            </li>
            <li>
              <span className="font-semibold text-slate-200">
                7. Probar conexión
              </span>
              <p className="mt-0.5">
                Si conectaste pero "Sync ahora" falla, pulsa{" "}
                <span className="font-mono">Probar conexión</span> — te
                dice paso por paso dónde se rompe (credenciales,
                refresh token, identidad, Drive API, scope, listado).
              </p>
            </li>
            <li className="rounded border border-rose-500/30 bg-rose-500/10 p-2">
              <span className="font-semibold text-rose-200">
                Si sigues con 403 después de activar la Drive API:
              </span>
              <ul className="mt-1 list-disc pl-4 text-rose-100/80">
                <li>
                  Revisa que la Drive API esté activada en EL MISMO
                  proyecto del paso 1.
                </li>
                <li>
                  Pulsa Desconectar + Conectar de nuevo — el access
                  token cacheado puede no tener las scopes nuevas.
                </li>
                <li>
                  En{" "}
                  <a
                    href="#"
                    onClick={(e) => {
                      e.preventDefault();
                      void api.openExternal(
                        "https://myaccount.google.com/permissions",
                      );
                    }}
                    className="underline"
                  >
                    myaccount.google.com/permissions
                  </a>{" "}
                  revoca el acceso de tu app, después reconecta.
                </li>
              </ul>
            </li>
          </ol>
        )}
      </div>
    </div>
  );
}

/** (v1.1.4) Importa los 3 formatos de export (CSV/TXT/JSON) y
 *  abre un modal para que el usuario elija qué re-descargar. Caso
/** (v3.5.0) Fila del importador de VAS-ACARS.
 *
 *  (v3.5.0 F2 v5) MSFS LOGBOOK.BIN removido — la carpeta
 *  `%APPDATA%\Microsoft Flight Simulator\LOGBOOK.BIN` ya no existe en
 *  MSFS 2024 (logbook movido al cloud) y el usuario reportó que el
 *  botón Simulador nunca tenía datos para importar. Mantenemos solo
 *  VAS-ACARS, que sí funciona consistentemente con tracking real. */
function MsfsLogbookRow({
  onFeedback,
}: {
  onFeedback: (msg: string | null) => void;
}) {
  const [importingVas, setImportingVas] = useState(false);
  const reload = useFlightLogStore((s) => s.reload);

  const onImportVas = async () => {
    console.log("[logbook] VAS-ACARS import clicked");
    setImportingVas(true);
    onFeedback(t("settings.msfs_logbook.importing"));
    try {
      const vas = await api.vasAcarsImport();
      console.log("[logbook] vas result:", vas);
      if (!vas.sourceDir || vas.candidatesFound === 0) {
        onFeedback(t("settings.msfs_logbook.no_logbook"));
        return;
      }
      onFeedback(
        t("settings.msfs_logbook.vas_success", {
          count: String(vas.importedCount),
          updated: String(vas.updatedCount),
          invalid: String(vas.skippedInvalid),
        }),
      );
      await reload();
    } catch (e) {
      console.error("[logbook] vas import error:", e);
      onFeedback(t("settings.msfs_logbook.error", { message: String(e) }));
    } finally {
      setImportingVas(false);
    }
  };

  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-xs text-slate-200">
            {t("settings.msfs_logbook.title")}
          </div>
          <p className="mt-0.5 text-[11px] text-slate-500">
            {t("settings.msfs_logbook.hint")}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            onClick={onImportVas}
            disabled={importingVas}
            title={t("settings.msfs_logbook.button_vas_hint")}
            className="inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-emerald-500/40 hover:text-emerald-200 disabled:opacity-50"
          >
            {importingVas ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <FileText className="h-3.5 w-3.5" />
            )}
            {t("settings.msfs_logbook.button_vas")}
          </button>
          <DeleteImportsButton onFeedback={onFeedback} />
        </div>
      </div>
    </div>
  );
}

/** (v3.5.0 F2) Botón "Borrar todos" para limpiar imports erróneos
 *  durante el prueba-y-error de VAS-ACARS. Usa `confirm()` nativo del
 *  browser (sin custom modal — es destructivo, queremos un blocker).
 *  Solo borra `source IN ('vas-acars', 'msfs-logbook')` — NUNCA toca
 *  los SimConnect captures (vuelos reales del usuario).
 */
function DeleteImportsButton({
  onFeedback,
}: {
  onFeedback: (msg: string | null) => void;
}) {
  const [deleting, setDeleting] = useState(false);
  const reload = useFlightLogStore((s) => s.reload);

  const onDelete = async () => {
    if (
      !window.confirm(
        t("settings.msfs_logbook.delete_confirm"),
      )
    ) {
      return;
    }
    setDeleting(true);
    onFeedback(null);
    try {
      const vas = await api.deleteFlightsBySource("vas-acars");
      const msfs = await api.deleteFlightsBySource("msfs-logbook");
      const total = vas.flightsDeleted + msfs.flightsDeleted;
      onFeedback(
        t("settings.msfs_logbook.delete_success", {
          total: String(total),
        }),
      );
      await reload();
    } catch (e) {
      onFeedback(`${t("common.error")}: ${String(e)}`);
    } finally {
      setDeleting(false);
    }
  };

  return (
    <button
      onClick={onDelete}
      disabled={deleting}
      className="inline-flex shrink-0 items-center gap-1 rounded-md border border-rose-500/30 bg-rose-500/5 px-2.5 py-1.5 text-xs text-rose-300 hover:border-rose-400/50 hover:bg-rose-500/10 hover:text-rose-200 disabled:opacity-50"
      title={t("settings.msfs_logbook.delete_hint")}
    >
      {deleting ? (
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
      ) : (
        <Trash2 className="h-3.5 w-3.5" />
      )}
      {t("settings.msfs_logbook.delete_button")}
    </button>
  );
}

/** (v1.1.4) Importa los 3 formatos de export (CSV/TXT/JSON) y
 *  abre un modal para que el usuario elija qué re-descargar. Caso
 *  de uso: formateo de PC, instalación nueva, vuelta a la rutina
 *  con su biblioteca anterior. Vive como modal lazy (`ImportInventoryModal`).
 */
function ImportInventoryRow({
  onFeedback,
}: {
  onFeedback: (msg: string | null) => void;
}) {
  const [pickingFile, setPickingFile] = useState(false);

  const onPick = async () => {
    setPickingFile(true);
    onFeedback(null);
    try {
      const path = await api.pickFilePath([
        { name: t("settings.import.file_filter"), extensions: ["csv", "txt", "json"] },
      ]);
      if (!path) {
        setPickingFile(false);
        return;
      }
      // El handler abre el modal — disparamos un evento global.
      window.dispatchEvent(
        new CustomEvent("msfs-addons:import-inventory", { detail: { path } }),
      );
    } catch (e) {
      onFeedback(`${t("common.error")}: ${String(e)}`);
    } finally {
      setPickingFile(false);
    }
  };

  return (
    <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-xs text-slate-200">{t("settings.import.title")}</div>
          <p className="mt-0.5 text-[11px] text-slate-500">
            {t("settings.import.hint")}
          </p>
        </div>
        <button
          onClick={onPick}
          disabled={pickingFile}
          className="inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-800 bg-slate-900/60 px-2.5 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 hover:text-brand-200 disabled:opacity-50"
        >
          {pickingFile ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <Upload className="h-3.5 w-3.5" />
          )}
          {t("common.import")}…
        </button>
      </div>
    </div>
  );
}
