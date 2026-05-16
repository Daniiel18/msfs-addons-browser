import { useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import { Download, Plane, Sparkles } from "lucide-react";
import type { UpdateInfo } from "../lib/types";

/**
 * Splash screen — pantalla de carga inicial.
 *
 * Diseño: minimal, profesional, undecorated (sin botones de
 * cerrar/minimizar — la idea es que el usuario NO interactúe
 * mientras carga).
 *
 * Estados especiales:
 *   · `appUpdate` populado → muestra un banner "Hay v0.X.Y" con
 *     botones "Instalar / Saltar". Si el usuario pulsa Instalar,
 *     `onInstallUpdate` corre la descarga + lanzado del installer
 *     (la app se cierra sola tras eso). El splash entra en modo
 *     "actualizando" con barra de progreso real.
 *   · `updateProgress` populado → muestra una barra de descarga
 *     mientras el installer baja.
 *
 * El componente NO inicia trabajo. App.tsx orquesta todo y le pasa
 * tareas + eventos.
 */
export function SplashScreen({
  tasks,
  appVersion,
  appUpdate,
  installing,
  updateProgress,
  onInstallUpdate,
  onSkipUpdate,
}: {
  tasks: SplashTask[];
  /** Versión actual de la app, leída de Tauri en runtime. */
  appVersion: string | null;
  /** Si está populado, mostramos el banner de actualización. */
  appUpdate: UpdateInfo | null;
  /** True mientras la descarga del installer está en curso. */
  installing: boolean;
  /** Bytes descargados / totales del installer en curso. */
  updateProgress: { downloadedBytes: number; totalBytes: number | null } | null;
  onInstallUpdate: () => void;
  onSkipUpdate: () => void;
}) {
  const total = tasks.length;
  const done = tasks.filter((t) => t.status === "done").length;
  const errored = tasks.filter((t) => t.status === "error");
  const current = tasks.find((t) => t.status === "pending");
  const pct = total === 0 ? 0 : Math.round((done / total) * 100);

  // Tagline aleatorio recordado durante el ciclo de vida del splash.
  const tagline = useMemo(() => pickTagline(), []);

  // Para que el "Hay update" aparezca sólo cuando el usuario aún
  // no decidió, internamente trackeamos si ya pulsó algo.
  const [pendingDecision, setPendingDecision] = useState(true);
  useEffect(() => {
    // Si appUpdate aparece nuevo, reseteamos la decisión a pendiente.
    if (appUpdate) setPendingDecision(true);
  }, [appUpdate]);

  const showUpdateBanner = !!appUpdate && pendingDecision && !installing;
  const downloadPct =
    updateProgress && updateProgress.totalBytes && updateProgress.totalBytes > 0
      ? Math.min(
          100,
          Math.round(
            (updateProgress.downloadedBytes / updateProgress.totalBytes) * 100,
          ),
        )
      : null;
  const mbDown = updateProgress
    ? (updateProgress.downloadedBytes / 1_048_576).toFixed(1)
    : null;
  const mbTotal =
    updateProgress?.totalBytes != null
      ? (updateProgress.totalBytes / 1_048_576).toFixed(1)
      : null;

  return (
    <div
      data-tauri-drag-region
      className="fixed inset-0 z-50 flex flex-col overflow-hidden bg-slate-950 text-slate-100"
    >
      {/* Fondo */}
      <div
        className="pointer-events-none absolute inset-0 bg-gradient-to-br from-slate-950 via-slate-900 to-slate-950"
        aria-hidden
      />
      <div
        className="pointer-events-none absolute inset-0 opacity-[0.04]"
        aria-hidden
        style={{
          backgroundImage:
            "radial-gradient(rgb(148, 163, 184) 1px, transparent 1px)",
          backgroundSize: "24px 24px",
        }}
      />
      <div
        className="pointer-events-none absolute left-1/2 top-[38%] h-72 w-72 -translate-x-1/2 -translate-y-1/2 rounded-full bg-emerald-500/10 blur-3xl"
        aria-hidden
      />

      <div className="relative flex flex-1 flex-col items-center justify-center px-8 pb-6 pt-2">
        {/* Logo con anillos animados */}
        <div className="relative mb-7 flex h-24 w-24 items-center justify-center">
          <motion.div
            animate={{ rotate: 360 }}
            transition={{ duration: 8, repeat: Infinity, ease: "linear" }}
            className="absolute inset-0 rounded-full"
            style={{
              background:
                "conic-gradient(from 0deg, transparent 0deg, transparent 270deg, rgb(52, 211, 153) 320deg, rgb(16, 185, 129) 360deg)",
              mask: "radial-gradient(circle, transparent 38px, black 39px, black 47px, transparent 48px)",
              WebkitMask:
                "radial-gradient(circle, transparent 38px, black 39px, black 47px, transparent 48px)",
            }}
          />
          <motion.div
            animate={{ rotate: -360 }}
            transition={{ duration: 14, repeat: Infinity, ease: "linear" }}
            className="absolute inset-2 rounded-full opacity-50"
            style={{
              background:
                "conic-gradient(from 90deg, transparent 0deg, transparent 290deg, rgb(56, 189, 248) 360deg)",
              mask: "radial-gradient(circle, transparent 30px, black 31px, black 36px, transparent 37px)",
              WebkitMask:
                "radial-gradient(circle, transparent 30px, black 31px, black 36px, transparent 37px)",
            }}
          />
          <div className="relative flex h-16 w-16 items-center justify-center rounded-2xl bg-slate-900 ring-1 ring-emerald-500/30 shadow-2xl shadow-emerald-500/20">
            <Plane className="h-8 w-8 text-emerald-300" strokeWidth={1.5} />
          </div>
        </div>

        <h1 className="text-center text-xl font-semibold tracking-tight text-slate-50">
          MSFS Addons Browser
        </h1>
        <p className="mt-1.5 text-center text-[11px] tracking-wide text-slate-500">
          {tagline}
        </p>

        {/* Banner update — sólo si hay update y el usuario aún no
            decidió, y no estamos descargando */}
        {showUpdateBanner && (
          <motion.div
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            className="mt-6 w-full max-w-[320px] rounded-xl border border-emerald-500/30 bg-emerald-500/10 px-3 py-2.5 ring-1 ring-emerald-500/20"
          >
            <div className="flex items-start gap-2 text-[11px]">
              <Sparkles className="mt-0.5 h-3.5 w-3.5 shrink-0 text-emerald-300" />
              <div className="flex-1">
                <div className="font-semibold text-emerald-100">
                  Nueva versión disponible: v{appUpdate!.latestVersion}
                </div>
                <div className="text-[10px] text-emerald-200/70">
                  Estás en v{appUpdate!.currentVersion}. Recomendamos
                  actualizar antes de seguir.
                </div>
              </div>
            </div>
            <div className="mt-2 flex gap-2">
              <button
                onClick={onInstallUpdate}
                disabled={!appUpdate!.assetUrl}
                className="inline-flex flex-1 items-center justify-center gap-1 rounded-md bg-emerald-500/30 px-2 py-1 text-[11px] font-medium hover:bg-emerald-500/40 disabled:opacity-50"
              >
                <Download className="h-3 w-3" /> Instalar ahora
              </button>
              <button
                onClick={() => {
                  setPendingDecision(false);
                  onSkipUpdate();
                }}
                className="rounded-md border border-slate-700 px-2 py-1 text-[11px] text-slate-300 hover:bg-slate-800"
              >
                Saltar
              </button>
            </div>
          </motion.div>
        )}

        {/* Barra de progreso del download del installer cuando
            installing está activo */}
        {installing && updateProgress && (
          <motion.div
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            className="mt-6 w-full max-w-[320px]"
          >
            <div className="mb-1 flex items-center justify-between text-[10px] text-emerald-200">
              <span>
                {downloadPct !== null
                  ? `Descargando v${appUpdate?.latestVersion ?? ""} · ${downloadPct}%`
                  : `Descargando v${appUpdate?.latestVersion ?? ""}`}
              </span>
              <span className="font-mono tabular-nums">
                {mbDown ?? "0"} {mbTotal ? `/ ${mbTotal}` : ""} MB
              </span>
            </div>
            <div className="h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
              <div
                className="h-full bg-emerald-400 transition-all duration-150"
                style={{ width: downloadPct !== null ? `${downloadPct}%` : "30%" }}
              />
            </div>
            {downloadPct === 100 && (
              <div className="mt-1 text-[10px] text-emerald-200/80">
                Descarga completa — lanzando el instalador. La app se cerrará
                en unos segundos.
              </div>
            )}
          </motion.div>
        )}

        {/* Barra de progreso de las tareas del bootstrap. Sólo
            visible cuando NO estamos en banner/installing — para
            no apilar barras en pantalla. */}
        {!showUpdateBanner && !installing && (
          <div className="mt-7 w-full max-w-[300px]">
            <div className="relative h-1 w-full overflow-hidden rounded-full bg-slate-800">
              <motion.div
                className="absolute inset-y-0 left-0 rounded-full bg-gradient-to-r from-emerald-500 via-emerald-400 to-cyan-400"
                initial={{ width: 0 }}
                animate={{ width: `${pct}%` }}
                transition={{ duration: 0.4, ease: "easeOut" }}
              />
              <motion.div
                className="absolute inset-y-0 w-12 bg-gradient-to-r from-transparent via-white/30 to-transparent"
                animate={{ x: ["-50%", "350%"] }}
                transition={{
                  duration: 2.2,
                  repeat: Infinity,
                  ease: "easeInOut",
                }}
              />
            </div>
            <div className="mt-3 flex h-4 items-center justify-center text-[11px] text-slate-400">
              {errored.length > 0 ? (
                <span className="text-rose-300">
                  {errored.length}{" "}
                  {errored.length === 1 ? "error" : "errores"} ·{" "}
                  {current?.label ?? "Finalizando"}
                </span>
              ) : current ? (
                <motion.span
                  key={current.label}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  transition={{ duration: 0.2 }}
                >
                  {current.label}…
                </motion.span>
              ) : (
                <motion.span
                  key="ready"
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  className="text-emerald-300"
                >
                  Listo
                </motion.span>
              )}
            </div>
          </div>
        )}
      </div>

      {/* Footer — versión real leída de Tauri en runtime */}
      <div className="relative flex items-center justify-between border-t border-slate-800/60 bg-slate-950/40 px-5 py-2.5 text-[10px] text-slate-600">
        <span className="font-mono tabular-nums">
          {done} / {total}
        </span>
        <span className="uppercase tracking-widest">
          v{appVersion ?? "?"}
        </span>
      </div>
    </div>
  );
}

export interface SplashTask {
  label: string;
  status: "pending" | "done" | "error";
  error?: string;
}

const TAGLINES = [
  "Preparando catálogo de addons…",
  "Sincronizando carpeta Community…",
  "Cargando aeropuertos del mundo…",
  "Calentando motores…",
  "Verificando últimas versiones…",
  "Trazando rutas en el mapa…",
];

function pickTagline(): string {
  return TAGLINES[Math.floor(Math.random() * TAGLINES.length)];
}
