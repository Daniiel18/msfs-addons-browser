/**
 * (v4.33.0) Sinónimos de vendor para la búsqueda de addons. El usuario
 * escribe "fenix" pero los folders/manifests usan "fnx"; "flybywire"
 * vs "fbw"; etc. Sin esto, buscar "fenix" sólo matchea el avión base
 * (title "Fenix Airbus A320") y NO sus liveries (folder
 * "fnx-aircraft-320-…"). Compartido por el grid (AddonsView) y el
 * Link Map.
 */
const VENDOR_SYNONYMS: Record<string, string[]> = {
  fenix: ["fnx"],
  fnx: ["fenix"],
  flybywire: ["fbw"],
  fbw: ["flybywire"],
  inibuilds: ["ini"],
  headwind: ["hdw", "headwindsim"],
  headwindsim: ["headwind"],
  aerosoft: ["asobo-aerosoft"],
  leonardo: ["fly-the-maddog", "maddog"],
  maddog: ["fly-the-maddog", "leonardo"],
};

/** Devuelve la query original + sus equivalentes de vendor. */
export function expandVendorQuery(q: string): string[] {
  if (!q) return [q];
  const out = new Set([q]);
  for (const [k, syns] of Object.entries(VENDOR_SYNONYMS)) {
    if (q.includes(k)) syns.forEach((s) => out.add(q.replace(k, s)));
  }
  return [...out];
}
