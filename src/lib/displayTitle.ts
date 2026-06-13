/**
 * (v4.26.0) Desambiguación de títulos duplicados de addons.
 *
 * Varios paquetes pueden declarar el MISMO title en su manifest:
 *   · `pmdg-aircraft-738` y `pmdg-tablet-73x` → ambos "737-800"
 *     (el segundo es el EFB/tablet de PMDG, no el avión).
 *   · Liveries sueltas tituladas genéricamente "Liveries".
 * Para esos grupos derivamos un sufijo distintivo del folder name:
 * quitamos los tokens comunes a todo el grupo (vendor, etc.) y
 * mostramos el resto prettificado: "737-800 (Tablet 73x)".
 *
 * Devuelve un Map folder → displayTitle SOLO para los que necesitan
 * sufijo; el resto usa su title tal cual.
 */
export function disambiguateTitles(
  items: { folderName: string; title: string }[],
): Map<string, string> {
  const out = new Map<string, string>();
  const groups = new Map<string, { folderName: string; title: string }[]>();
  for (const it of items) {
    const key = it.title.trim().toLowerCase();
    const arr = groups.get(key) ?? [];
    arr.push(it);
    groups.set(key, arr);
  }
  for (const members of groups.values()) {
    if (members.length < 2) continue;
    // Tokens comunes a TODOS los folders del grupo (en minúsculas) —
    // típicamente el vendor ("pmdg") — no aportan distinción.
    const tokenSets = members.map(
      (m) =>
        new Set(
          m.folderName
            .toLowerCase()
            .split(/[-_\s]+/)
            .filter(Boolean),
        ),
    );
    const common = new Set(
      [...tokenSets[0]].filter((t) => tokenSets.every((s) => s.has(t))),
    );
    for (const m of members) {
      const distinct = m.folderName
        .split(/[-_\s]+/)
        .filter((t) => t && !common.has(t.toLowerCase()))
        .join(" ");
      const suffix = prettify(distinct || m.folderName, 26);
      out.set(m.folderName, `${m.title} (${suffix})`);
    }
  }
  return out;
}

function prettify(s: string, maxLen: number): string {
  const pretty = s
    .split(/\s+/)
    .map((w) =>
      // Códigos cortos tipo "73x"/"738" en mayúsculas; palabras
      // normales con inicial mayúscula.
      w.length <= 4 ? w.toUpperCase() : w.charAt(0).toUpperCase() + w.slice(1),
    )
    .join(" ");
  return pretty.length > maxLen ? `${pretty.slice(0, maxLen - 1)}…` : pretty;
}
