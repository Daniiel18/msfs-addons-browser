import { countryName } from "./countryName";
import { t } from "./i18n";
import type { LinkConditions } from "./types";

/** Códigos de un campo: separados como se quiera. */
export const parseCodes = (text: string): string[] =>
  text
    .toUpperCase()
    .split(/[^A-Z0-9]+/)
    .filter(Boolean);

const joined = (list: string[], word: string) =>
  list.length < 2 ? list.join("") : `${list.slice(0, -1).join(", ")} ${word} ${list[list.length - 1]}`;

/** (05/11) «Solo si: país Brasil o Canadá y avión B738». */
export function conditionsText(c: LinkConditions): string {
  const any = (list: string[]) => joined(list, t("linkmap.cond.or"));
  const parts = [
    c.country.length > 0 && t("linkmap.cond.part.country", { list: any(c.country.map((code) => countryName(code) ?? code)) }),
    c.airport.length > 0 && t("linkmap.cond.part.airport", { list: any(c.airport) }),
    c.aircraft.length > 0 && t("linkmap.cond.part.aircraft", { list: any(c.aircraft) }),
    c.airline.length > 0 && t("linkmap.cond.part.airline", { list: any(c.airline) }),
  ].filter((part): part is string => !!part);
  return t("linkmap.cond.sentence", { rest: joined(parts, t("linkmap.cond.and")) });
}
