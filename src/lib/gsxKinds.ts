import type { GsxParkingKind } from "./types";

/** (19/10) Color de cada tipo de posición de un perfil GSX. Lo comparten
 *  los pines del Scenery map y la lista de gates de la tarjeta. */
export const GSX_KIND_COLORS: Record<GsxParkingKind, string> = {
  gate: "#35cdb2",
  parking: "#38bdf8",
  ramp: "#94a3b8",
  cargo: "#f59e0b",
  military: "#a3a3a3",
  pad: "#a78bfa",
  other: "#cbd5e1",
};
