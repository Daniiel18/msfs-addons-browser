import { useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Activity,
  AlertCircle,
  Archive,
  Bell,
  Compass,
  Database,
  Download,
  FileJson,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  HardDrive,
  Info,
  Loader2,
  MinusSquare,
  Plane,
  Power,
  RotateCcw,
  Settings as SettingsIcon,
  Trash2,
  X,
} from "lucide-react";
import type { ExportFormat } from "../lib/types";
import { useSettingsStore } from "../stores/useSettingsStore";
import { useSimBriefStore } from "../stores/useSimBriefStore";
import { api } from "../lib/tauri";

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
      setFeedback(`Limpiadas ${n} entradas de caché.`);
    } catch (e) {
      setFeedback(`Error: ${String(e)}`);
    } finally {
      setClearing(false);
    }
  };

  const onReset = async () => {
    if (
      !window.confirm(
        "¿Restablecer todas las preferencias a sus valores por defecto? El SimBrief Pilot ID y el dataset de aeropuertos no se tocan.",
      )
    )
      return;
    setResetting(true);
    setFeedback(null);
    try {
      const n = await resetSettings();
      setFeedback(`Restablecidas ${n} preferencias a sus valores por defecto.`);
    } catch (e) {
      setFeedback(`Error: ${String(e)}`);
    } finally {
      setResetting(false);
    }
  };

  const openPath = (path: string | null) => {
    if (!path) return;
    api.openLocalPath(path).catch((e) => setFeedback(`No se pudo abrir: ${String(e)}`));
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
        `Backup creado: ${r.outputPath} · ${r.packageCount} paquetes (${(r.totalBytes / 1_000_000).toFixed(0)} MB) en ${(r.elapsedMs / 1000).toFixed(1)} s.`,
      );
    } catch (e) {
      setFeedback(`Error de backup: ${String(e)}`);
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
      setFeedback(`Exportadas ${r.rowCount} filas a ${r.outputPath}`);
    } catch (e) {
      setFeedback(`Error de export: ${String(e)}`);
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
                  Configuración
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

              <Section title="General" icon={<Power className="h-3.5 w-3.5" />}>
                <Toggle
                  label="Arrancar con Windows"
                  hint="La app se abre automáticamente al iniciar sesión."
                  checked={settings.autostartEnabled}
                  onChange={(v) => setAutostart(v)}
                />
                <Toggle
                  label="Comprobar updates al arrancar"
                  hint="Verifica nuevas versiones de la app en GitHub al iniciar."
                  checked={settings.checkUpdatesOnStart}
                  onChange={(v) => setBoolean("checkUpdatesOnStart", v)}
                />
                <Toggle
                  label="Minimizar a la bandeja al cerrar"
                  hint="Al pulsar la X, la app se oculta en la bandeja del sistema (junto al reloj). Para salir, click derecho en el icono → Salir."
                  checked={settings.minimizeToTray}
                  onChange={(v) => setMinimizeToTray(v)}
                />
              </Section>

              <Section title="Vuelos" icon={<Plane className="h-3.5 w-3.5" />}>
                <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
                  <div className="text-xs text-slate-200">SimBrief Pilot ID</div>
                  <p className="mt-0.5 text-[11px] text-slate-500">
                    En SimBrief: <span className="font-mono">Account → Pilot ID</span>.
                    Sin esto, los vuelos no se descargan.
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
                      {savingPilot ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Guardar"}
                    </button>
                    <button
                      onClick={() => refreshSimBrief()}
                      disabled={!pilotId || savingPilot}
                      title="Refrescar el último OFP"
                      className="rounded-md border border-slate-800 bg-slate-900/60 px-3 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 disabled:opacity-40"
                    >
                      Refrescar
                    </button>
                  </div>
                </div>
                <div className="mt-2 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2 text-[11px] text-slate-400">
                  <Activity className="mr-1 inline h-3 w-3 text-emerald-300" />
                  SimConnect (vuelos reales): el watcher arranca con la app y
                  se conecta automáticamente cuando MSFS está corriendo.
                  Cada vuelo registrado aparece en{" "}
                  <span className="font-medium text-emerald-200">FlightBook</span>.
                </div>
              </Section>

              <Section title="Mostrar en mapa (FlightBook)" icon={<Bell className="h-3.5 w-3.5" />}>
                <Toggle
                  label="Líneas de SimBrief"
                  hint="Vuelos planificados (cyan, dasheado) en el mapa de FlightBook."
                  checked={settings.showSimbriefLines}
                  onChange={(v) => setBoolean("showSimbriefLines", v)}
                />
                <Toggle
                  label="Líneas de SimConnect"
                  hint="Vuelos reales registrados (verde, sólido) en el mapa de FlightBook."
                  checked={settings.showSimconnectLines}
                  onChange={(v) => setBoolean("showSimconnectLines", v)}
                />
              </Section>

              <Section title="Carpetas" icon={<FolderOpen className="h-3.5 w-3.5" />}>
                <PathRow
                  label="Carpeta Community"
                  hint="MSFS lee los addons desde aquí."
                  path={settings.communityPath}
                  onOpen={() => openPath(settings.communityPath)}
                />
                <PathRow
                  label="Logs"
                  hint="Logs rotados por día — útiles para diagnóstico."
                  path={settings.logsPath}
                  onOpen={() => openPath(settings.logsPath)}
                />
                <PathRow
                  label="Datos de la app"
                  hint="Base de datos SQLite + caché GSX/updates."
                  path={settings.appDataPath}
                  onOpen={() => openPath(settings.appDataPath)}
                />
              </Section>

              <Section title="Backup" icon={<Archive className="h-3.5 w-3.5" />}>
                <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
                  <div className="flex items-center gap-1.5 text-xs text-slate-200">
                    <Archive className="h-3 w-3 text-slate-500" />
                    Comprimir carpeta Community
                  </div>
                  <p className="mt-0.5 text-[11px] text-slate-500">
                    Crea un .zip con timestamp en la carpeta que elijas. Útil
                    antes de actualizar MSFS o probar packs experimentales.
                    El proceso puede tardar varios minutos en colecciones grandes.
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
                      Crear backup…
                    </button>
                  </div>
                </div>
              </Section>

              <Section title="Exportar inventario" icon={<Download className="h-3.5 w-3.5" />}>
                <div className="rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5">
                  <div className="text-xs text-slate-200">
                    Lista de addons instalados
                  </div>
                  <p className="mt-0.5 text-[11px] text-slate-500">
                    Exporta título, autor, versión, ICAO, tamaño y fecha de
                    cada paquete. Elige formato:
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

              <Section title="Almacenamiento" icon={<HardDrive className="h-3.5 w-3.5" />}>
                <ActionRow
                  icon={<Database className="h-3 w-3 text-slate-500" />}
                  label="Limpiar cachés"
                  hint="Borra resultados cacheados de GSX y de chequeos de versión. Útil cuando una update esperada no aparece."
                  buttonLabel="Limpiar"
                  buttonIcon={<Trash2 className="h-3.5 w-3.5" />}
                  buttonTone="rose"
                  busy={clearing}
                  onClick={onClearCaches}
                />
                <ActionRow
                  icon={<RotateCcw className="h-3 w-3 text-slate-500" />}
                  label="Restablecer preferencias"
                  hint="Devuelve los toggles a sus valores por defecto. No afecta tu Pilot ID, addons instalados, ni dataset de aeropuertos."
                  buttonLabel="Reset"
                  buttonIcon={<MinusSquare className="h-3.5 w-3.5" />}
                  buttonTone="amber"
                  busy={resetting}
                  onClick={onReset}
                />
              </Section>

              <Section title="Tour de bienvenida" icon={<Compass className="h-3.5 w-3.5" />}>
                <ActionRow
                  icon={<Compass className="h-3 w-3 text-slate-500" />}
                  label="Volver a ver el tour"
                  hint="Te llevamos por el Dashboard, Buscar, Mapa, Addons y Configuración. 6 pasos, ~30 segundos."
                  buttonLabel="Lanzar"
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

              <Section title="Acerca de" icon={<Info className="h-3.5 w-3.5" />}>
                <div className="space-y-1 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2.5 text-[11px] text-slate-400">
                  <div>
                    <span className="text-slate-500">Versión:</span>{" "}
                    <span className="font-mono text-slate-200">
                      {appVersion ?? "—"}
                    </span>
                  </div>
                  <div>
                    <span className="text-slate-500">GitHub:</span>{" "}
                    <a
                      href="https://github.com/Daniiel18/msfs-addons-browser"
                      target="_blank"
                      rel="noopener noreferrer"
                      className="text-brand-300 hover:underline"
                    >
                      Daniiel18/msfs-addons-browser
                    </a>
                  </div>
                </div>
              </Section>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
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
          <div className="mt-1 text-[10px] italic text-slate-600">No detectada</div>
        )}
      </div>
      <button
        onClick={onOpen}
        disabled={!path}
        className="inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-800 px-2.5 py-1.5 text-xs text-slate-300 hover:border-brand-500/40 disabled:cursor-not-allowed disabled:opacity-40"
      >
        <FolderOpen className="h-3.5 w-3.5" />
        Abrir
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
