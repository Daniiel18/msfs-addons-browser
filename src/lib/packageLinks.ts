import { t } from "./i18n";
import type { CommunityPackage, UninstallReport } from "./types";

/** Qué decir tras desinstalar si lo que se fue era un enlace. */
export function keptNote(report: UninstallReport): string | undefined {
  const kept = report.keptTargets ?? [];
  return kept.length > 0 ? t("pkg.uninstall_link.kept", { path: kept.join(" · ") }) : undefined;
}

/** ¿Vive en `Community2024`? */
export const inSecondFolder = (p: CommunityPackage) => /[\\/]Community2024[\\/]/i.test(p.installPath);
