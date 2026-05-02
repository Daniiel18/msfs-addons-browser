/**
 * Comparación de versiones semver tolerante. Réplica del helper de
 * `updates.rs` en backend para que el frontend pueda decidir
 * "¿esta versión del catálogo es más nueva que la instalada?" sin
 * un round-trip al backend.
 *
 * Reglas:
 *   · Acepta `v1.2.3`, `1.2.3`, `1.2`, `1` — rellena ceros a la
 *     derecha hasta llegar a tres componentes.
 *   · Tokens no numéricos se comparan lexicográficamente.
 *   · Empate entre componentes prefiere igualdad.
 *
 * Usar esta función en lugar de `localeCompare` directo en strings:
 * `"1.10.0".localeCompare("1.9.0")` devuelve negativo porque '1' < '9'
 * cuando no se interpreta como número — exactamente el bug que
 * queremos evitar.
 */
export function compareVersions(a: string, b: string): number {
  const pa = parseLenient(a);
  const pb = parseLenient(b);
  if (!pa || !pb) {
    // Sin parseo válido caemos a comparación lexicográfica — es
    // determinista pero no necesariamente "correcta".
    return a.trim().localeCompare(b.trim());
  }
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i] - pb[i];
  }
  return 0;
}

/**
 * `latest` estrictamente mayor que `installed`. Strings vacíos
 * devuelven `false` — no hay update si una de las dos versiones es
 * desconocida.
 */
export function isNewer(latest: string | null | undefined, installed: string | null | undefined): boolean {
  if (!latest || !installed) return false;
  const l = latest.trim();
  const i = installed.trim();
  if (!l || !i) return false;
  return compareVersions(l, i) > 0;
}

function parseLenient(input: string): [number, number, number] | null {
  const s = input.trim().replace(/^v/i, "");
  if (!s) return null;
  const parts = s.split(".").map((p) => p.trim());
  const nums: number[] = [];
  for (const part of parts) {
    // Tomamos los dígitos iniciales del componente — `1.6.5-beta`
    // se trata como `1.6.5`, suficiente para detección de updates.
    const m = part.match(/^(\d+)/);
    if (!m) return null;
    nums.push(parseInt(m[1], 10));
  }
  while (nums.length < 3) nums.push(0);
  return [nums[0], nums[1], nums[2]];
}
