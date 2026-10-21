import { t } from "./i18n";

/**
 * (19/10) Nombre legible de un componente de `maintenance_log`. Los cobros
 * del mantenimiento hecho en TechLog llegan como `techlog:<tipo>:<detalle>`
 * (ver `techlog::sync_charges`); el resto son ids de SimFleet.
 */
export function maintComponentLabel(component: string): string {
  // (19/10) Etapa 3: los cobros del Tech Log de SimFleet llegan como
  // `mx:<tipo>:<detalle>` (mismo formato que los de TechLog).
  const own = component.startsWith("mx:");
  if (!own && !component.startsWith("techlog:")) return t(`economy.comp.${component}`);
  const [, kind, ...rest] = component.split(":");
  const label = maintChargeLabel(kind, rest.join(":"));
  return own ? label.replace(/^TechLog/, "SimFleet") : label;
}

function maintChargeLabel(kind: string, detail: string): string {
  switch (kind) {
    case "part_replace":
      return t("techlog.charge.part_replace", { name: detail });
    case "part_repair":
      return t("techlog.charge.part_repair", { name: detail });
    case "service":
      return t("techlog.charge.service", { name: detail });
    case "defect":
      return t("techlog.charge.defect", { name: detail });
    case "check":
      return t("techlog.charge.check", { name: detail });
    default:
      return `TechLog · ${detail || kind}`;
  }
}
