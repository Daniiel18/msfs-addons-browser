import { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import maplibregl from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import { AlertTriangle, Check, Loader2, Trash2 } from "lucide-react";
import type { FlightLogEntry, FlightTrackPoint } from "../lib/types";
import { api } from "../lib/tauri";
import { t } from "../lib/i18n";
import { segmentTrackCoords, smoothCatmullRom } from "../lib/smooth";
import { cleanAtcType } from "../lib/aircraft";

/**
 * (v4.23.0) Modal de VUELO INCOMPLETO — aparece al abrir la app cuando
 * hay vuelos cerrados como 'partial' sin revisar (la PC se apagó / el
 * sim murió a mitad de vuelo y el tracking quedó cortado). Muestra el
 * routemap con el track real recorrido + las métricas (ruta, número de
 * vuelo, callsign, altitud máx, etc.) y pregunta:
 *   · **Mantener** → el vuelo pasa a `completed` y aparece en el
 *     FlightBook con su track parcial.
 *   · **Eliminar** → se borra de la DB.
 * Si hay varios, se revisan uno por uno (contador "1 / N").
 */
export function IncompleteFlightModal() {
  const [queue, setQueue] = useState<FlightLogEntry[]>([]);
  const [busy, setBusy] = useState(false);

  // Carga inicial + recarga cuando el watcher detecta otro huérfano.
  useEffect(() => {
    let cancelled = false;
    const load = () => {
      api
        .listIncompleteFlights()
        .then((rows) => {
          if (!cancelled) setQueue(rows);
        })
        .catch(() => {});
    };
    load();
    let unlisten: (() => void) | null = null;
    import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen("flightlog://incomplete", () => load()).then((fn) => {
          unlisten = fn;
        }),
      )
      .catch(() => {});
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  const entry = queue[0] ?? null;

  const resolve = async (keep: boolean) => {
    if (!entry || busy) return;
    setBusy(true);
    try {
      if (keep) {
        await api.keepPartialFlight(entry.id);
      } else {
        await api.deleteFlightLogEntry(entry.id);
      }
    } catch (e) {
      console.error("[IncompleteFlightModal] resolve falló:", e);
    } finally {
      setBusy(false);
      setQueue((q) => q.slice(1));
    }
  };

  if (!entry) return null;
  return (
    <IncompleteFlightCard
      key={entry.id}
      entry={entry}
      total={queue.length}
      busy={busy}
      onResolve={resolve}
    />
  );
}

function IncompleteFlightCard({
  entry,
  total,
  busy,
  onResolve,
}: {
  entry: FlightLogEntry;
  total: number;
  busy: boolean;
  onResolve: (keep: boolean) => void;
}) {
  const mapContainerRef = useRef<HTMLDivElement | null>(null);

  // Mini-mapa con el track real recorrido (mismo pipeline del routemap).
  useEffect(() => {
    const container = mapContainerRef.current;
    if (!container) return;
    const map = new maplibregl.Map({
      container,
      style: {
        version: 8,
        sources: {
          "carto-dark": {
            type: "raster",
            tiles: [
              "https://cartodb-basemaps-a.global.ssl.fastly.net/dark_all/{z}/{x}/{y}.png",
            ],
            tileSize: 256,
            attribution: "© OpenStreetMap · © CARTO",
          },
        },
        layers: [
          { id: "carto-dark-layer", type: "raster", source: "carto-dark" },
        ],
      },
      center: [entry.originLon ?? 0, entry.originLat ?? 20],
      zoom: 3,
      attributionControl: false,
      interactive: false,
    });
    map.on("load", () => {
      api
        .getFlightTrack(entry.id)
        .then((pts: FlightTrackPoint[]) => {
          const lines = segmentTrackCoords(
            pts.map((p) => ({ lon: p.lon, lat: p.lat, ts: p.ts })),
          )
            .filter((s) => s.length >= 2)
            .map((s) => smoothCatmullRom(s, 8));
          if (lines.length === 0) return;
          map.addSource("inc-track", {
            type: "geojson",
            data: {
              type: "Feature",
              properties: {},
              geometry: { type: "MultiLineString", coordinates: lines },
            },
          });
          map.addLayer({
            id: "inc-track-line",
            type: "line",
            source: "inc-track",
            layout: { "line-cap": "round", "line-join": "round" },
            paint: { "line-color": "#fbbf24", "line-width": 2.4 },
          });
          const all = lines.flat();
          const lons = all.map((c) => c[0]);
          const lats = all.map((c) => c[1]);
          map.fitBounds(
            [
              [Math.min(...lons), Math.min(...lats)],
              [Math.max(...lons), Math.max(...lats)],
            ],
            { padding: 30, duration: 0, maxZoom: 9 },
          );
        })
        .catch(() => {});
    });
    return () => {
      map.remove();
    };
  }, [entry.id, entry.originLat, entry.originLon]);

  const startedLocal = (() => {
    try {
      return new Date(entry.startedAt).toLocaleString([], {
        day: "2-digit",
        month: "short",
        hour: "2-digit",
        minute: "2-digit",
      });
    } catch {
      return entry.startedAt;
    }
  })();

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      className="fixed inset-0 z-[70] flex items-center justify-center bg-slate-950/80 px-4 backdrop-blur-sm"
    >
      <motion.div
        initial={{ opacity: 0, y: 10, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        className="w-full max-w-lg overflow-hidden rounded-2xl border border-amber-500/30 bg-slate-950 shadow-2xl ring-1 ring-amber-500/20"
      >
        {/* Header */}
        <div className="flex items-center justify-between gap-2 border-b border-slate-800 px-4 py-3">
          <div className="flex items-center gap-2">
            <AlertTriangle className="h-4 w-4 text-amber-300" />
            <h2 className="text-sm font-semibold text-slate-100">
              {t("fb.incomplete.title")}
            </h2>
          </div>
          {total > 1 && (
            <span className="rounded bg-slate-800 px-1.5 py-0.5 text-[10px] text-slate-400">
              1 / {total}
            </span>
          )}
        </div>

        {/* Mini routemap con el track recorrido */}
        <div ref={mapContainerRef} className="h-48 w-full bg-slate-900" />

        {/* Detalle */}
        <div className="space-y-3 p-4">
          <p className="text-xs leading-relaxed text-slate-400">
            {t("fb.incomplete.body")}
          </p>
          <div className="flex items-baseline gap-3 font-mono">
            <span className="text-lg font-bold text-slate-100">
              {entry.originIcao ?? "?"}
            </span>
            <span className="text-amber-400">→</span>
            <span className="text-lg font-bold text-slate-100">
              {entry.destinationIcao ?? "?"}
            </span>
            {(entry.callsign || entry.flightNumber) && (
              <span className="text-sm font-semibold text-sky-300">
                {entry.callsign ?? entry.flightNumber}
              </span>
            )}
          </div>
          <div className="grid grid-cols-3 gap-1.5 text-center">
            <div className="rounded-lg bg-slate-900/70 px-2 py-1.5">
              <div className="text-[9px] uppercase tracking-wide text-slate-500">
                {t("fb.incomplete.started")}
              </div>
              <div className="font-mono text-[11px] text-slate-200">
                {startedLocal}
              </div>
            </div>
            <div className="rounded-lg bg-slate-900/70 px-2 py-1.5">
              <div className="text-[9px] uppercase tracking-wide text-slate-500">
                {t("fb.incomplete.max_alt")}
              </div>
              <div className="font-mono text-[11px] text-slate-200">
                {entry.maxAltitudeFt != null
                  ? `${entry.maxAltitudeFt.toLocaleString()} ft`
                  : "—"}
              </div>
            </div>
            <div className="rounded-lg bg-slate-900/70 px-2 py-1.5">
              <div className="text-[9px] uppercase tracking-wide text-slate-500">
                {t("fb.incomplete.aircraft")}
              </div>
              <div className="truncate font-mono text-[11px] text-slate-200">
                {cleanAtcType(entry.aircraftAtcType) ??
                  entry.aircraftModel ??
                  entry.aircraftTitle ??
                  "—"}
              </div>
            </div>
          </div>

          {/* Acciones */}
          <div className="flex items-center justify-end gap-2 pt-1">
            <button
              onClick={() => onResolve(false)}
              disabled={busy}
              className="inline-flex items-center gap-1.5 rounded-md border border-rose-500/40 bg-rose-500/10 px-3 py-1.5 text-xs font-medium text-rose-200 hover:bg-rose-500/20 disabled:opacity-50"
            >
              <Trash2 className="h-3.5 w-3.5" />
              {t("fb.incomplete.delete")}
            </button>
            <button
              onClick={() => onResolve(true)}
              disabled={busy}
              className="inline-flex items-center gap-1.5 rounded-md bg-emerald-500 px-3 py-1.5 text-xs font-semibold text-emerald-950 hover:bg-emerald-400 disabled:opacity-50"
            >
              {busy ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
              ) : (
                <Check className="h-3.5 w-3.5" />
              )}
              {t("fb.incomplete.keep")}
            </button>
          </div>
        </div>
      </motion.div>
    </motion.div>
  );
}
