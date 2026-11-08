import { canonDev } from "./addonMatch";
import { getActiveLocale, t } from "./i18n";
import type { CommunityPackage } from "./types";

/** (06/11) Filtros y grupos de Instalados y del Mapa de escenarios. */

export interface PackageGroup<T> {
  key: string;
  label: string;
  items: T[];
}

export interface FilterOption {
  value: string;
  label: string;
}

const STUDIO_WORDS = /(studios|studio|simulations|simulation|designs|design|development|team)+$/;

/**
 * Un creador con varias grafías cuenta una vez: «SayIntentions.AI» y
 * «SayIntentionsAI», «MK-STUDIOS» y «mkstudios». Sin creador en el manifest
 * queda la casa del nombre de la carpeta.
 */
export function creatorKey(p: CommunityPackage): string {
  const name = canonDev(p.creator);
  if (!name) return canonDev(p.folderName.split("-")[0]);
  const short = name.replace(STUDIO_WORDS, "");
  return short.length >= 4 ? short : name;
}

/** Un addon firmado por otro también es de la casa que lleva en la carpeta. */
export const matchesCreator = (p: CommunityPackage, key: string): boolean =>
  creatorKey(p) === key || canonDev(p.folderName).startsWith(key);

const byLabel = (a: FilterOption, b: FilterOption) => a.label.localeCompare(b.label);

/** Nombre de cada creador: su grafía más repetida. */
function creatorNames(packages: CommunityPackage[]): Map<string, string> {
  const seen = new Map<string, Map<string, number>>();
  for (const p of packages) {
    const key = creatorKey(p);
    const label = p.creator?.trim() || p.folderName.split("-")[0];
    const spellings = seen.get(key) ?? new Map<string, number>();
    spellings.set(label, (spellings.get(label) ?? 0) + 1);
    seen.set(key, spellings);
  }
  const names = new Map<string, string>();
  for (const [key, spellings] of seen) {
    names.set(key, [...spellings].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0][0]);
  }
  return names;
}

export function creatorOptions(packages: CommunityPackage[]): FilterOption[] {
  return [...creatorNames(packages)]
    .map(([value, name]) => ({
      value,
      label: `${name} (${packages.filter((p) => matchesCreator(p, value)).length})`,
    }))
    .sort(byLabel);
}

export function packageTags(p: CommunityPackage): string[] {
  try {
    const list: unknown = JSON.parse(p.userTagsJson ?? "[]");
    return Array.isArray(list) ? list.filter((tag): tag is string => typeof tag === "string") : [];
  } catch {
    return [];
  }
}

export function tagOptions(packages: CommunityPackage[]): FilterOption[] {
  const count = new Map<string, number>();
  for (const p of packages) {
    for (const tag of packageTags(p)) count.set(tag, (count.get(tag) ?? 0) + 1);
  }
  return [...count].map(([value, n]) => ({ value, label: `${value} (${n})` })).sort(byLabel);
}

/** Grupos con nombre primero; lo que no tiene dato, al final. */
export function groupItems<T>(
  items: T[],
  keyOf: (item: T) => string,
  labelOf: (key: string) => string,
  emptyLabel: string,
): PackageGroup<T>[] {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const key = keyOf(item);
    groups.set(key, [...(groups.get(key) ?? []), item]);
  }
  return [...groups]
    .map(([key, list]) => ({ key, label: key ? labelOf(key) : emptyLabel, items: list }))
    .sort((a, b) => Number(!a.key) - Number(!b.key) || a.label.localeCompare(b.label));
}

export function groupByCreator<T>(
  items: T[],
  packageOf: (item: T) => CommunityPackage,
): PackageGroup<T>[] {
  const names = creatorNames(items.map(packageOf));
  return groupItems(
    items,
    (item) => creatorKey(packageOf(item)),
    (key) => names.get(key) ?? key,
    t("addons.group.no_creator"),
  );
}

const monthName = (key: string) =>
  new Date(`${key}-15T12:00:00Z`).toLocaleDateString(getActiveLocale(), { month: "long", year: "numeric" });

/** Por mes de la última actualización, lo más nuevo primero. */
export function groupByMonth<T>(items: T[], packageOf: (item: T) => CommunityPackage): PackageGroup<T>[] {
  return groupItems(items, (item) => packageOf(item).lastUpdate?.slice(0, 7) ?? "", monthName, t("addons.group.no_date")).sort(
    (a, b) => Number(!a.key) - Number(!b.key) || b.key.localeCompare(a.key),
  );
}

export function readChoice<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const saved = localStorage.getItem(key);
    return allowed.find((value) => value === saved) ?? fallback;
  } catch {
    return fallback;
  }
}

export function saveChoice(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Sin almacenamiento, vale para esta sesión.
  }
}
