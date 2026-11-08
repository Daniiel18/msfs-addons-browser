import type { CommunityPackage } from "./types";

/** (04/11) Etapa 9 — orden de Instalados y del Mapa de escenarios. */
export type AddonSort = "name" | "updated" | "rating";

export const ADDON_SORTS: AddonSort[] = ["name", "updated", "rating"];

const label = (p: CommunityPackage) => p.airportName ?? p.title;
const byName = (a: CommunityPackage, b: CommunityPackage) => label(a).localeCompare(label(b));

/** Lo más nuevo y lo mejor valorado primero; lo que no tiene dato, al final. */
export function compareAddons(sort: AddonSort): (a: CommunityPackage, b: CommunityPackage) => number {
  if (sort === "updated") return (a, b) => (b.lastUpdate ?? "").localeCompare(a.lastUpdate ?? "") || byName(a, b);
  if (sort === "rating") return (a, b) => (b.rating ?? 0) - (a.rating ?? 0) || byName(a, b);
  return byName;
}

export function readSort(key: string): AddonSort {
  try {
    const saved = localStorage.getItem(key);
    return ADDON_SORTS.find((s) => s === saved) ?? "name";
  } catch {
    return "name";
  }
}

export function saveSort(key: string, sort: AddonSort) {
  try {
    localStorage.setItem(key, sort);
  } catch {
    // Sin almacenamiento: el orden vive en memoria.
  }
}
