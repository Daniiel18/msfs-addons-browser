import { create } from "zustand";
import { api } from "../lib/tauri";
import { useCommunityStore } from "./useCommunityStore";
import type { GsxProfile } from "../lib/types";

/**
 * (v6.2.64) Oferta de perfiles GSX al instalar un escenario de aeropuerto.
 *
 * Cuando aparece una carpeta NUEVA en Community con ICAO (un escenario recién
 * instalado), consultamos flightsim.to por perfiles GSX de ese aeropuerto y, si
 * hay, mostramos un modal con nombre, imagen, descargas y estrellas. El clic
 * abre esa página en el embebido para descargar (los perfiles GSX no se pueden
 * bajar headless por el Cloudflare de flightsim.to).
 *
 * Mecánica igual que [[useFpsNoticesStore]]: mapa de carpetas conocidas
 * persistido; la primera vez SIEMBRA sin ofrecer (lo ya instalado no es nuevo).
 */

const KNOWN_KEY = "simfleet.gsxoffer.known.v1";

export interface GsxOffer {
  icao: string;
  airportTitle: string;
  /** Desarrollador del escenario instalado — para marcar el perfil compatible. */
  sceneryCreator: string | null;
  profiles: GsxProfile[];
}

interface GsxOfferState {
  offer: GsxOffer | null;
  dismiss: () => void;
  start: () => void;
}

function loadKnown(): Record<string, true> | null {
  try {
    const raw = localStorage.getItem(KNOWN_KEY);
    return raw ? (JSON.parse(raw) as Record<string, true>) : null;
  } catch {
    return null;
  }
}
function saveKnown(v: Record<string, true>) {
  try {
    localStorage.setItem(KNOWN_KEY, JSON.stringify(v));
  } catch {
    /* ignore */
  }
}

let started = false;
let checking = false;

export const useGsxOfferStore = create<GsxOfferState>((set, get) => ({
  offer: null,
  dismiss: () => set({ offer: null }),
  start: () => {
    if (started) return;
    started = true;

    const check = async () => {
      if (checking) return;
      const pkgs = useCommunityStore.getState().packages;
      if (pkgs.length === 0) return;
      checking = true;
      try {
        let known = loadKnown();
        if (known == null) {
          // Primera vez: sembrar todo lo instalado, sin ofrecer nada.
          const seeded: Record<string, true> = {};
          for (const p of pkgs) if (p.folderName) seeded[p.folderName] = true;
          saveKnown(seeded);
          return;
        }
        // (v6.2.68) PODAR: quitar de `known` las carpetas que ya NO están
        // instaladas. Así, si el usuario desinstala y REINSTALA un escenario,
        // vuelve a contar como nuevo → el modal reaparece.
        const currentFolders = new Set(pkgs.map((p) => p.folderName));
        let pruned = false;
        for (const f of Object.keys(known)) {
          if (!currentFolders.has(f)) {
            delete known[f];
            pruned = true;
          }
        }
        if (pruned) saveKnown(known);
        // Carpetas nuevas con ICAO = escenarios de aeropuerto recién instalados.
        const fresh = pkgs.filter(
          (p) => p.folderName && !known![p.folderName] && p.icao,
        );
        // Registrar TODAS las nuevas (con o sin ICAO) para no re-procesarlas.
        if (pkgs.some((p) => p.folderName && !known![p.folderName])) {
          const next = { ...known };
          for (const p of pkgs) if (p.folderName) next[p.folderName] = true;
          saveKnown(next);
        }
        if (fresh.length === 0 || get().offer) return;
        // Mostramos el modal SIEMPRE para el primer escenario nuevo con ICAO,
        // tenga o no perfiles GSX (si no hay, el modal lo indica).
        const p = fresh[0];
        const icao = (p.icao || "").toUpperCase();
        if (!icao) return;
        let profiles: GsxProfile[] = [];
        try {
          profiles = await api.gsxLookup(icao);
        } catch (e) {
          console.warn("gsxLookup (offer) falló:", e);
        }
        set({
          offer: {
            icao,
            airportTitle: p.title || icao,
            sceneryCreator: p.creator ?? null,
            profiles,
          },
        });
      } catch (e) {
        console.warn("gsx offer check falló:", e);
      } finally {
        checking = false;
      }
    };

    void check();
    let lastPkgs = useCommunityStore.getState().packages;
    useCommunityStore.subscribe((s) => {
      if (s.packages !== lastPkgs) {
        lastPkgs = s.packages;
        void check();
      }
    });
  },
}));
