import { isFromDeveloper } from "./addonMatch";
import type { CommunityPackage, GsxLocalProfile } from "./types";

export type GsxStatus = "none" | "ok" | "orphan" | "duplicate" | "other_scenery";

export interface GsxStatusInfo {
  status: GsxStatus;
  profiles: GsxLocalProfile[];
  /** Con `other_scenery`, el desarrollador que nombra el perfil. */
  otherDev?: string;
}

/**
 * (05/11) Estado del perfil GSX de un aeropuerto. «De otro escenario» solo sale
 * si el nombre del perfil cita a otro desarrollador del catálogo: un nombre sin
 * pista, como `cyeg-ee7mjp.ini`, no dice nada y cuenta como correcto.
 */
export function gsxStatusFor(
  icao: string,
  sceneries: CommunityPackage[],
  profiles: GsxLocalProfile[],
): GsxStatusInfo {
  const own = profiles.filter((p) => p.icao === icao.toUpperCase());
  if (own.length === 0) return { status: "none", profiles: own };
  if (own.length > 1) return { status: "duplicate", profiles: own };
  if (sceneries.length === 0) return { status: "orphan", profiles: own };
  const dev = own[0].sceneryDev;
  if (dev && !sceneries.some((p) => isFromDeveloper(p, dev))) {
    return { status: "other_scenery", profiles: own, otherDev: dev };
  }
  return { status: "ok", profiles: own };
}
