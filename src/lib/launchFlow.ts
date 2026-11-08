import { t } from "./i18n";
import type { LaunchDetectedApp, LaunchSequence, LaunchStep } from "./types";

/**
 * (02/11) Etapa 6 — secuencia de arranque: ayudas puras del front.
 */

/** Secuencia para el avión del OFP: la que tiene ese tipo; si no, la «por
 *  defecto»; si no, la primera. */
export function pickSequence(
  sequences: LaunchSequence[],
  aircraftIcao: string | null | undefined,
): { seq: LaunchSequence | null; reason: "aircraft" | "default" | "first" | null } {
  const type = aircraftIcao?.trim().toUpperCase();
  const byType = type ? sequences.find((s) => s.aircraft.includes(type)) : undefined;
  if (byType) return { seq: byType, reason: "aircraft" };
  const def = sequences.find((s) => s.isDefault);
  if (def) return { seq: def, reason: "default" };
  return sequences.length > 0 ? { seq: sequences[0], reason: "first" } : { seq: null, reason: null };
}

export function newStepId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `s${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

const base = (kind: LaunchStep["kind"], name: string, target: string): LaunchStep => ({
  id: newStepId(),
  kind,
  name,
  target,
  args: "",
  process: [],
  open: "normal",
  wait: "seconds",
  waitSeconds: 2,
  close: "stay",
});

export const stepFromApp = (a: LaunchDetectedApp): LaunchStep => ({
  ...base("app", a.name, a.target),
  args: a.args,
  process: a.process,
  open: a.open,
  wait: a.wait,
  waitSeconds: a.waitSeconds,
  close: a.close,
});

export const simStep = (): LaunchStep => ({ ...base("sim", "MSFS", ""), waitSeconds: 3 });

/** Nombre de un .exe o carpeta elegidos a mano. */
function leafName(path: string): string {
  const leaf = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  return leaf.replace(/\.(exe|bat|cmd|ps1)$/i, "");
}

export const appStep = (path: string): LaunchStep => ({ ...base("app", leafName(path), path), close: "exit" });
export const folderStep = (path: string): LaunchStep => base("folder", leafName(path), path);
export const scriptStep = (path: string): LaunchStep => base("script", leafName(path), path);

export function webStep(url: string): LaunchStep {
  let name = url;
  try {
    name = new URL(url).hostname.replace(/^www\./, "");
  } catch {
    /* el back lo valida */
  }
  return base("web", name, url);
}

/** ¿Dirección web válida para un paso? */
export function isWebUrl(u: string): boolean {
  try {
    const p = new URL(u.trim());
    return p.protocol === "http:" || p.protocol === "https:";
  } catch {
    return false;
  }
}

/** Texto de un error del back (`not_found`, `uac_cancelled`, `failed:…`). */
export function launchError(code: string | null | undefined): string {
  if (!code) return "";
  const known = [
    "not_found",
    "uac_cancelled",
    "no_sim",
    "test_env",
    "bad_web",
    "busy",
    "empty",
    "name_required",
    "bad_app",
    "bad_folder",
    "bad_script",
    "script_timeout",
    "script_permission",
    "one_sim",
    "too_many_steps",
  ];
  // `script_failed:<código>:<lo último que escribió>`.
  const failed = /^script_failed:(-?\d+):([\s\S]*)$/.exec(code);
  if (failed) {
    return [t("launch.err.script_failed", { code: failed[1] }), failed[2]].filter(Boolean).join(" · ");
  }
  const key = known.find((k) => code === k || code.endsWith(k));
  if (key) return t(`launch.err.${key}`);
  return code.startsWith("failed:") ? code.slice("failed:".length) : code;
}

/** Texto corto de la espera antes de un paso. */
export function waitLabel(step: Pick<LaunchStep, "wait" | "waitSeconds">, first: boolean): string {
  if (step.wait === "menu") return t("launch.wait.menu");
  if (step.wait === "loaded") return t("launch.wait.loaded");
  if (step.waitSeconds <= 0) return first ? t("launch.wait.start") : t("launch.wait.now");
  return t("launch.wait.seconds", { n: String(step.waitSeconds) });
}
