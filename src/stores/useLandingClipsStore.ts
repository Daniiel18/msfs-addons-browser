import { create } from "zustand";
import { api, isTauri } from "../lib/tauri";
import type { FlightLogEntry, LandingClip } from "../lib/types";

/**
 * (8.4.x) Clips de aterrizaje compartidos (FlightBook, "Video del
 * aterrizaje"). Se leen una vez y se recargan cuando cambia el FlightBook
 * (el clip se enriquece con su vuelo ~30 s después del toque).
 */
interface LandingClipsState {
  clips: LandingClip[];
  reload: () => Promise<void>;
  start: () => void;
}

let started = false;

export const useLandingClipsStore = create<LandingClipsState>((set, get) => ({
  clips: [],
  reload: async () => {
    try {
      set({ clips: await api.listLandingClips() });
    } catch {
      /* sin clips (demo, carpeta no disponible) */
    }
  },
  start: () => {
    if (started) return;
    started = true;
    void get().reload();
    if (!isTauri) return;
    void import("@tauri-apps/api/event").then(({ listen }) => {
      void listen("flightlog://changed", () => void get().reload());
    });
  },
}));

/** `recordedAt` del backend: "YYYY-MM-DD HH:MM:SS" en UTC (sin zona). */
function parseUtc(s: string | null | undefined): number {
  if (!s) return NaN;
  const iso = /[zZ]|[+-]\d\d:?\d\d$/.test(s) ? s : `${s.replace(" ", "T")}Z`;
  return Date.parse(iso);
}

/**
 * El clip del aterrizaje de `entry`: el que guarda su `flightId` (clips
 * nuevos) o, para los de antes, el grabado en su destino entre el despegue
 * y poco después de cerrar el vuelo.
 */
export function clipForFlight(clips: LandingClip[], entry: FlightLogEntry): LandingClip | null {
  const own = clips.find((c) => c.flightId === entry.id);
  if (own) return own;
  const dest = entry.destinationIcao?.toUpperCase();
  const from = parseUtc(entry.startedAt);
  const to = entry.endedAt ? parseUtc(entry.endedAt) + 10 * 60_000 : Date.now();
  if (!dest || Number.isNaN(from)) return null;
  const cands = clips.filter((c) => {
    if (c.isTest || c.flightId != null || c.airportIcao?.toUpperCase() !== dest) return false;
    const at = parseUtc(c.recordedAt);
    return !Number.isNaN(at) && at >= from && at <= to;
  });
  return cands.sort((a, b) => parseUtc(b.recordedAt) - parseUtc(a.recordedAt))[0] ?? null;
}
