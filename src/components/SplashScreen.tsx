import { useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  Download,
  Fingerprint,
  KeyRound,
  Loader2,
  Lock,
  Sparkles,
} from "lucide-react";
import type { UpdateInfo } from "../lib/types";
import { t } from "../lib/i18n";
import { BrandMark } from "./BrandMark";

/**
 * Splash screen — pantalla de carga inicial.
 *
 * (19/10) Rediseño con el estilo de la nueva UI (tokens sf-*, acento
 * turquesa) + BLOQUEO DE INICIO: la app no se abre hasta verificar al
 * usuario de Windows (Windows Hello o su contraseña de Windows). La carga
 * sigue en segundo plano mientras tanto.
 *
 * Estados especiales:
 *   · `appUpdate` populado → banner "Hay vX.Y.Z" con Instalar / Saltar.
 *   · `updateProgress` populado → barra de descarga del instalador.
 *   · `lock.required` → tarjeta de desbloqueo.
 *
 * El componente NO inicia trabajo. App.tsx orquesta todo y le pasa
 * tareas + eventos.
 */
export interface SplashLock {
  /** La app sigue bloqueada (falta verificar al usuario de Windows). */
  required: boolean;
  /** null = aún comprobando si Windows Hello está disponible. */
  helloAvailable: boolean | null;
  username: string;
  busy: boolean;
  error: string | null;
  /** La carga terminó y sólo falta desbloquear. */
  waiting: boolean;
  onHello: () => void;
  onPassword: (password: string) => void;
}

export function SplashScreen({
  tasks,
  appVersion,
  appUpdate,
  installing,
  updateProgress,
  onInstallUpdate,
  onSkipUpdate,
  lock,
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
  /** (v2.1.0) Callback recibe la preferencia de auto-restart. */
  onInstallUpdate: (autoRestart: boolean) => void;
  onSkipUpdate: () => void;
  /** (19/10) Bloqueo de inicio con la cuenta de Windows. */
  lock?: SplashLock;
}) {
  // (v2.1.0) Auto-restart tras instalar (default true).
  const [autoRestart, setAutoRestart] = useState(true);
  const [showChangelog, setShowChangelog] = useState(false);
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
    // La ventana se redimensiona a 480x640 al inicio del bootstrap (ver
    // App.tsx); aquí sólo llenamos lo que hay.
    <div
      data-tauri-drag-region
      className="fixed inset-0 z-50 overflow-hidden bg-sf-bg text-sf-text"
    >
      <div data-tauri-drag-region className="relative flex h-full w-full flex-col">
        {/* Fondo: brillo turquesa + trazos de ruta */}
        <div
          className="pointer-events-none absolute -top-24 left-1/2 h-80 w-80 -translate-x-1/2 rounded-full bg-brand-500/15 blur-3xl"
          aria-hidden
        />
        <SplashRouteArt className="pointer-events-none absolute inset-x-0 bottom-10 h-40 w-full text-brand-400/20" />

        <div className="relative flex flex-1 flex-col items-center justify-center px-8 pb-6 pt-4">
          {/* Logo */}
          <div className="relative mb-6 flex h-24 w-24 items-center justify-center">
            <motion.div
              animate={{ rotate: 360 }}
              transition={{ duration: 7, repeat: Infinity, ease: "linear" }}
              className="absolute inset-0 rounded-full"
              style={{
                background:
                  "conic-gradient(from 0deg, transparent 0deg, transparent 250deg, rgb(var(--sf-accent)) 360deg)",
                mask: "radial-gradient(circle, transparent 42px, black 43px, black 47px, transparent 48px)",
                WebkitMask:
                  "radial-gradient(circle, transparent 42px, black 43px, black 47px, transparent 48px)",
              }}
            />
            {/* (19/10) Logo nuevo (monograma S), el mismo del icono de la app. */}
            <BrandMark className="relative h-16 w-16 rounded-2xl shadow-2xl shadow-brand-500/30" />
          </div>

          <h1 className="text-center text-2xl font-semibold tracking-tight text-sf-text">
            SimFleet
          </h1>
          <p className="mt-1 text-center text-xs text-sf-muted">{tagline}</p>

          {/* Banner update */}
          {showUpdateBanner && (
            <motion.div
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              className="mt-6 w-full max-w-[340px] rounded-xl border border-brand-500/30 bg-sf-surface p-3"
            >
              <div className="flex items-start gap-2 text-xs">
                <Sparkles className="mt-0.5 h-4 w-4 shrink-0 text-brand-300" />
                <div className="flex-1">
                  <div className="font-semibold text-sf-text">
                    {t("update.banner.new_version", { version: appUpdate!.latestVersion })}
                  </div>
                  <div className="text-[11px] text-sf-muted">
                    {t("update.banner.body", { current: appUpdate!.currentVersion })}
                  </div>
                </div>
              </div>

              {appUpdate!.notesMarkdown && (
                <div className="mt-2">
                  <button
                    onClick={() => setShowChangelog((v) => !v)}
                    className="inline-flex items-center gap-1 text-[11px] text-brand-300 hover:text-brand-200"
                  >
                    {showChangelog ? "▲" : "▼"} {t("update.banner.view_changes")}
                  </button>
                  {showChangelog && (
                    <div className="mt-1 max-h-32 overflow-y-auto whitespace-pre-wrap rounded border border-sf-border bg-sf-bg/60 px-2 py-1.5 text-left font-mono text-[11px] leading-relaxed text-sf-text/90">
                      {appUpdate!.notesMarkdown.slice(0, 4000)}
                      {appUpdate!.notesMarkdown.length > 4000 && (
                        <>
                          …{"\n"}
                          <a
                            href={appUpdate!.releaseUrl}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="text-brand-300 underline"
                          >
                            {t("update.banner.see_full_release")}
                          </a>
                        </>
                      )}
                    </div>
                  )}
                </div>
              )}

              <label className="mt-2 flex cursor-pointer items-center gap-1.5 text-[11px] text-sf-text/90">
                <input
                  type="checkbox"
                  checked={autoRestart}
                  onChange={(e) => setAutoRestart(e.target.checked)}
                  className="accent-[rgb(var(--sf-accent))]"
                />
                {t("update.banner.auto_restart")}
              </label>

              <div className="mt-2 flex gap-2">
                <button
                  onClick={() => onInstallUpdate(autoRestart)}
                  disabled={!appUpdate!.assetUrl}
                  className="inline-flex flex-1 items-center justify-center gap-1 rounded-md bg-brand-500 px-2 py-1.5 text-[11px] font-semibold text-slate-950 hover:bg-brand-400 disabled:opacity-50"
                >
                  <Download className="h-3 w-3" /> {t("update.banner.install_now")}
                </button>
                <button
                  onClick={() => {
                    setPendingDecision(false);
                    onSkipUpdate();
                  }}
                  className="rounded-md border border-sf-border px-2 py-1.5 text-[11px] text-sf-text/90 hover:bg-sf-raised"
                >
                  {t("common.skip")}
                </button>
              </div>
            </motion.div>
          )}

          {/* Descarga del instalador */}
          {installing && updateProgress && (
            <motion.div
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              className="mt-6 w-full max-w-[320px]"
            >
              <div className="mb-1 flex items-center justify-between text-[11px] text-brand-200">
                <span>
                  {downloadPct !== null
                    ? t("update.progress.downloading_pct", {
                        version: appUpdate?.latestVersion ?? "",
                        pct: downloadPct,
                      })
                    : t("update.progress.downloading", {
                        version: appUpdate?.latestVersion ?? "",
                      })}
                </span>
                <span className="font-mono tabular-nums">
                  {mbDown ?? "0"} {mbTotal ? `/ ${mbTotal}` : ""} MB
                </span>
              </div>
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-sf-raised">
                <div
                  className="h-full bg-brand-400 transition-all duration-150"
                  style={{ width: downloadPct !== null ? `${downloadPct}%` : "30%" }}
                />
              </div>
              {downloadPct === 100 && (
                <div className="mt-1 text-[11px] text-brand-200/80">
                  {t("update.progress.complete")}
                </div>
              )}
            </motion.div>
          )}

          {/* Progreso del bootstrap */}
          {!showUpdateBanner && !installing && (
            <div className="mt-6 w-full max-w-[300px]">
              <div className="relative h-1 w-full overflow-hidden rounded-full bg-sf-raised">
                <motion.div
                  className="absolute inset-y-0 left-0 rounded-full bg-gradient-to-r from-brand-600 to-brand-400"
                  initial={{ width: 0 }}
                  animate={{ width: `${pct}%` }}
                  transition={{ duration: 0.4, ease: "easeOut" }}
                />
              </div>
              <div className="mt-2.5 flex h-4 items-center justify-center text-[11px] text-sf-muted">
                {errored.length > 0 ? (
                  <span className="text-rose-300">
                    {errored.length}{" "}
                    {errored.length === 1 ? t("common.error").toLowerCase() : t("common.errors")} ·{" "}
                    {current?.label ?? t("splash.finishing")}
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
                  <span className="text-brand-300">
                    {lock?.required ? t("lock.ready") : t("common.ready")}
                  </span>
                )}
              </div>
            </div>
          )}

          {/* (19/10) Desbloqueo con la cuenta de Windows */}
          {lock?.required && !installing && <LockCard lock={lock} />}
        </div>

        {/* Footer — versión real leída de Tauri en runtime */}
        <div className="relative flex items-center justify-between border-t border-sf-border/60 px-5 py-2.5 text-[11px] text-sf-muted/80">
          <span className="font-mono tabular-nums">
            {done} / {total}
          </span>
          <span className="uppercase tracking-widest">v{appVersion ?? "?"}</span>
        </div>
      </div>
    </div>
  );
}

/** (19/10) Tarjeta de desbloqueo: Windows Hello o contraseña de Windows. */
function LockCard({ lock }: { lock: SplashLock }) {
  const [usePassword, setUsePassword] = useState(false);
  const [password, setPassword] = useState("");
  const passwordMode = lock.helloAvailable === false || usePassword;

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      className={`mt-6 w-full max-w-[320px] rounded-2xl border bg-sf-surface/90 p-4 shadow-xl shadow-black/20 ${
        lock.waiting ? "border-brand-500/40" : "border-sf-border"
      }`}
    >
      <div className="flex items-center gap-2 text-sm font-semibold text-sf-text">
        <Lock className="h-4 w-4 text-brand-300" />
        {t("lock.title")}
      </div>
      <p className="mt-1 text-xs text-sf-muted">{t("lock.hint")}</p>
      {lock.username && (
        <p className="mt-0.5 text-[11px] text-sf-muted/80">
          {t("lock.user", { user: lock.username })}
        </p>
      )}

      {passwordMode ? (
        <form
          className="mt-3 flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (!password || lock.busy) return;
            lock.onPassword(password);
            setPassword("");
          }}
        >
          <div className="relative flex-1">
            <KeyRound className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-sf-muted" />
            <input
              type="password"
              autoFocus
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={t("lock.password_placeholder")}
              autoComplete="current-password"
              className="w-full rounded-lg border border-sf-border bg-sf-bg/70 py-2 pl-8 pr-2 text-sm text-sf-text placeholder:text-sf-muted/70 focus:border-brand-500/60 focus:outline-none"
            />
          </div>
          <button
            type="submit"
            disabled={!password || lock.busy}
            className="inline-flex items-center gap-1.5 rounded-lg bg-brand-500 px-3 text-xs font-semibold text-slate-950 hover:bg-brand-400 disabled:opacity-50"
          >
            {lock.busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
            {t("lock.unlock")}
          </button>
        </form>
      ) : (
        <button
          onClick={lock.onHello}
          disabled={lock.busy || lock.helloAvailable === null}
          className="mt-3 inline-flex w-full items-center justify-center gap-2 rounded-lg bg-brand-500 px-3 py-2 text-sm font-semibold text-slate-950 hover:bg-brand-400 disabled:opacity-60"
        >
          {lock.busy || lock.helloAvailable === null ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Fingerprint className="h-4 w-4" />
          )}
          {t("lock.unlock_hello")}
        </button>
      )}

      {!passwordMode && lock.helloAvailable && (
        <button
          onClick={() => setUsePassword(true)}
          className="mt-2 w-full text-center text-[11px] text-sf-muted underline-offset-2 hover:text-sf-text hover:underline"
        >
          {t("lock.use_password")}
        </button>
      )}

      {lock.error && <p className="mt-2 text-xs text-rose-300">{lock.error}</p>}
    </motion.div>
  );
}

/** Trazos de ruta decorativos (SVG puro). */
function SplashRouteArt({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 480 160" preserveAspectRatio="xMidYMid slice" className={className} fill="none" aria-hidden>
      <g stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeDasharray="2 7">
        <path d="M-10 140 C 120 40, 300 30, 490 90" />
        <path d="M40 160 C 160 90, 330 80, 480 20" />
      </g>
      <g fill="currentColor">
        <circle cx="490" cy="90" r="3" />
        <circle cx="480" cy="20" r="3" />
      </g>
    </svg>
  );
}

export interface SplashTask {
  label: string;
  status: "pending" | "done" | "error";
  error?: string;
}

/** (v3.4.0) Taglines i18n. Las keys son `splash.tagline.0..5`. */
const TAGLINE_COUNT = 6;
function pickTagline(): string {
  const idx = Math.floor(Math.random() * TAGLINE_COUNT);
  return t(`splash.tagline.${idx}`);
}
