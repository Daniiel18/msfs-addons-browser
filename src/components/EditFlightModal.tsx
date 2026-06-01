import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Loader2, Save, X } from "lucide-react";
import { api } from "../lib/tauri";
import type { FlightLogEntry } from "../lib/types";

interface Props {
  entry: FlightLogEntry;
  onClose: () => void;
  onSaved: () => void;
}

/**
 * Modal para editar manualmente un vuelo ya cerrado (v0.1.25).
 *
 * Usado cuando el watcher detectó mal alguna métrica:
 *   · Block time inflado por una pausa larga que SimConnect no
 *     capturó (versión vieja sin el "Pause" event subscription, o
 *     pausa del SO/Tab-Alt+F4 que no llega al sim).
 *   · Gate erróneo — el fallback "Stand · 053° 659m de HKJK" es un
 *     offset desde el centro del aeropuerto cuando el watcher no
 *     pudo detectar el parking real. El usuario sabe que aterrizó
 *     en el "9" y puede corregir.
 *   · Pasajeros + carga: sin integración GSX, son manuales por ahora.
 *   · Fuel: automático cuando SimConnect estaba conectado al OUT,
 *     pero el usuario puede editar si quiere reportar carga
 *     diferente (ej. fuel para reservas).
 */
export function EditFlightModal({ entry, onClose, onSaved }: Props) {
  // Estado del form — todos como strings para input controlado.
  const [hours, setHours] = useState(() =>
    entry.flightTimeS != null
      ? Math.floor(entry.flightTimeS / 3600).toString()
      : "",
  );
  const [minutes, setMinutes] = useState(() =>
    entry.flightTimeS != null
      ? Math.floor((entry.flightTimeS % 3600) / 60).toString()
      : "",
  );
  const [passengers, setPassengers] = useState(() =>
    entry.passengers != null ? entry.passengers.toString() : "",
  );
  const [cargoKg, setCargoKg] = useState(() =>
    entry.cargoKg != null ? entry.cargoKg.toString() : "",
  );
  const [fuelUsedKg, setFuelUsedKg] = useState(() =>
    entry.fuelUsedKg != null ? entry.fuelUsedKg.toString() : "",
  );
  const [depGate, setDepGate] = useState(() => entry.departureGate ?? "");
  const [arrGate, setArrGate] = useState(() => entry.arrivalGate ?? "");
  const [registration, setRegistration] = useState(
    () => entry.aircraftRegistration ?? "",
  );
  // (v4.0.0 — P6.2) Origen / destino editables. Útil cuando el watcher
  // detectó un ICAO erróneo o cuando un import VAS trae el aeropuerto
  // mal parseado. El usuario corrige tipeando un ICAO (3-4 letras).
  const [originIcao, setOriginIcao] = useState(() => entry.originIcao ?? "");
  const [destIcao, setDestIcao] = useState(() => entry.destinationIcao ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // ESC cierra.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !saving) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, saving]);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      // Construimos el payload sólo con campos que el usuario tocó
      // (distinguibles porque su string difiere del valor original).
      // Para tiempos, comparamos el total en segundos.
      const totalSec = (() => {
        const h = parseInt(hours || "0", 10);
        const m = parseInt(minutes || "0", 10);
        if (Number.isNaN(h) || Number.isNaN(m)) return null;
        if (h === 0 && m === 0 && (hours === "" || minutes === "")) return null;
        return h * 3600 + m * 60;
      })();
      const numOrNull = (s: string): number | null => {
        if (s.trim() === "") return null;
        const n = parseInt(s, 10);
        return Number.isNaN(n) ? null : n;
      };

      const input = {
        flightTimeS:
          totalSec !== null && totalSec !== entry.flightTimeS
            ? totalSec
            : undefined,
        passengers:
          numOrNull(passengers) !== entry.passengers
            ? numOrNull(passengers)
            : undefined,
        cargoKg:
          numOrNull(cargoKg) !== entry.cargoKg ? numOrNull(cargoKg) : undefined,
        fuelUsedKg:
          numOrNull(fuelUsedKg) !== entry.fuelUsedKg
            ? numOrNull(fuelUsedKg)
            : undefined,
        departureGate:
          depGate !== (entry.departureGate ?? "") ? depGate : undefined,
        arrivalGate:
          arrGate !== (entry.arrivalGate ?? "") ? arrGate : undefined,
        aircraftRegistration:
          registration !== (entry.aircraftRegistration ?? "")
            ? registration
            : undefined,
        // (v4.0.0 — P6.2) Origen / destino — solo enviamos si el
        // usuario realmente cambió el valor (string compare con el
        // ICAO original). Backend uppercases automáticamente.
        originIcao:
          originIcao.trim().toUpperCase() !== (entry.originIcao ?? "").toUpperCase()
            ? originIcao.trim()
            : undefined,
        destinationIcao:
          destIcao.trim().toUpperCase() !== (entry.destinationIcao ?? "").toUpperCase()
            ? destIcao.trim()
            : undefined,
      };
      await api.updateFlightLogEntry(entry.id, input);
      onSaved();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        exit={{ opacity: 0 }}
        transition={{ duration: 0.12 }}
        onClick={() => !saving && onClose()}
        // (v4.0.0 — P6.2 fix iter 3) El SCROLL vive en el backdrop,
        // no en el body del modal. Razón: la ventana de SimFleet es
        // 540px alta — incluso con max-h del modal y body scrolleable,
        // WebView2 + framer-motion en algunos casos no honran bien la
        // cadena flex/grid + overflow-y-auto. Approach más robusto:
        // backdrop scrollea el modal entero cuando no cabe → en
        // pantallas grandes se centra normal, en chicas se scrollea
        // verticalmente hasta llegar al footer con Guardar/Cancelar.
        // Patrón estándar de cualquier dialog web (Tailwind UI,
        // Headless UI, Radix Dialog).
        className="fixed inset-0 z-[65] flex items-start justify-center overflow-y-auto bg-slate-950/75 p-4 backdrop-blur-sm"
      >
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 8 }}
          transition={{ duration: 0.16 }}
          onClick={(e) => e.stopPropagation()}
          // `my-auto` centra verticalmente cuando hay espacio; cuando
          // el modal es más alto que el viewport, los margins son 0
          // automáticamente y el contenido se ve completo via scroll
          // del backdrop. Se quitó `scale` de la animación porque
          // interfería con el cálculo de altura inicial al montar.
          className="my-auto w-[min(560px,calc(100vw-2rem))] overflow-hidden rounded-2xl border border-amber-500/40 bg-slate-950 shadow-2xl ring-1 ring-amber-500/20"
        >
          <header className="flex items-start justify-between gap-3 border-b border-slate-800 px-5 py-3">
            <div>
              <h3 className="text-sm font-semibold text-amber-200">
                Editar vuelo · {entry.originIcao ?? "?"} → {entry.destinationIcao ?? "?"}
              </h3>
              <p className="mt-0.5 text-[11px] text-slate-400">
                Ajusta lo que el watcher detectó mal — block time, gates,
                pasajeros, carga, fuel. Deja vacío lo que no quieras tocar.
              </p>
            </div>
            <button
              onClick={onClose}
              disabled={saving}
              className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100 disabled:opacity-50"
            >
              <X className="h-4 w-4" />
            </button>
          </header>

          <div className="space-y-3 p-4">
            <Field label="Block time">
              <div className="flex items-center gap-1.5">
                <NumInput
                  value={hours}
                  onChange={setHours}
                  placeholder="h"
                  min={0}
                  max={48}
                  className="w-14"
                />
                <span className="text-xs text-slate-500">h</span>
                <NumInput
                  value={minutes}
                  onChange={setMinutes}
                  placeholder="m"
                  min={0}
                  max={59}
                  className="w-14"
                />
                <span className="text-xs text-slate-500">m</span>
                <span className="ml-2 text-[10px] text-slate-500">
                  (block-to-block real)
                </span>
              </div>
            </Field>

            {/* (v4.0.0 — P6.2) Origen + destino ICAO editables.
                Útil cuando el watcher detectó mal el spawn o cuando
                un import VAS trae el ICAO con typo. El backend hace
                .to_uppercase() antes de persistir. */}
            <div className="grid grid-cols-2 gap-3">
              <Field label="Origen (ICAO)">
                <input
                  type="text"
                  value={originIcao}
                  onChange={(e) => setOriginIcao(e.target.value.toUpperCase())}
                  placeholder="Ej. KJFK, LFPG, SEQM"
                  maxLength={4}
                  className="w-full rounded-md border border-slate-700 bg-slate-900/60 px-2 py-1.5 text-sm font-mono uppercase tracking-wide text-slate-100 focus:border-amber-400 focus:outline-none"
                />
              </Field>
              <Field label="Destino (ICAO)">
                <input
                  type="text"
                  value={destIcao}
                  onChange={(e) => setDestIcao(e.target.value.toUpperCase())}
                  placeholder="Ej. EGLL, KLAX, OMDB"
                  maxLength={4}
                  className="w-full rounded-md border border-slate-700 bg-slate-900/60 px-2 py-1.5 text-sm font-mono uppercase tracking-wide text-slate-100 focus:border-amber-400 focus:outline-none"
                />
              </Field>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <Field label="Pasajeros">
                <NumInput
                  value={passengers}
                  onChange={setPassengers}
                  placeholder="—"
                  min={0}
                  max={999}
                />
              </Field>
              <Field label="Carga (kg)">
                <NumInput
                  value={cargoKg}
                  onChange={setCargoKg}
                  placeholder="—"
                  min={0}
                  max={500_000}
                />
              </Field>
            </div>

            <Field label="Fuel consumido (kg)">
              <NumInput
                value={fuelUsedKg}
                onChange={setFuelUsedKg}
                placeholder="—"
                min={0}
                max={500_000}
              />
              <p className="mt-1 text-[10px] text-slate-500">
                Si SimConnect estaba conectado al pushback, ya se capturó
                automático. Edita aquí para corregir.
              </p>
            </Field>

            <Field label="Matrícula del avión">
              <input
                type="text"
                value={registration}
                onChange={(e) => setRegistration(e.target.value.toUpperCase())}
                placeholder="Ej. N404DX, D-AIZA, PK-GHG"
                maxLength={10}
                className="w-full rounded-md border border-slate-700 bg-slate-900/60 px-2 py-1.5 text-sm uppercase text-slate-100 focus:border-amber-400 focus:outline-none"
              />
              <p className="mt-1 text-[10px] text-slate-500">
                Tail number real. Usado por planespotters.net para mostrar la
                foto del avión en el FlightBook.
              </p>
            </Field>

            <div className="grid grid-cols-2 gap-3">
              <Field label="Gate salida (parking)">
                <input
                  type="text"
                  value={depGate}
                  onChange={(e) => setDepGate(e.target.value)}
                  placeholder="Ej. A12, B23"
                  className="w-full rounded-md border border-slate-700 bg-slate-900/60 px-2 py-1.5 text-sm text-slate-100 focus:border-amber-400 focus:outline-none"
                />
              </Field>
              <Field label="Gate llegada (parking)">
                <input
                  type="text"
                  value={arrGate}
                  onChange={(e) => setArrGate(e.target.value)}
                  placeholder="Ej. 9, D5"
                  className="w-full rounded-md border border-slate-700 bg-slate-900/60 px-2 py-1.5 text-sm text-slate-100 focus:border-amber-400 focus:outline-none"
                />
              </Field>
            </div>

            {error && (
              <div className="rounded-md border border-rose-500/30 bg-rose-500/10 p-2 text-xs text-rose-200">
                {error}
              </div>
            )}
          </div>

          <footer className="flex items-center justify-between gap-2 border-t border-slate-800 px-5 py-3">
            <p className="text-[10px] text-slate-500">
              Los cambios se guardan en la DB local.
            </p>
            <div className="flex items-center gap-2">
              <button
                onClick={onClose}
                disabled={saving}
                className="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800 disabled:opacity-50"
              >
                Cancelar
              </button>
              <button
                onClick={handleSave}
                disabled={saving}
                className="inline-flex items-center gap-1.5 rounded-md bg-amber-500 px-3 py-1.5 text-xs font-medium text-amber-950 hover:bg-amber-400 disabled:opacity-50"
              >
                {saving ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <Save className="h-3.5 w-3.5" />
                )}
                Guardar
              </button>
            </div>
          </footer>
        </motion.div>
      </motion.div>
    </AnimatePresence>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div>
      <label className="mb-1 block text-[10px] uppercase tracking-wide text-slate-400">
        {label}
      </label>
      {children}
    </div>
  );
}

function NumInput({
  value,
  onChange,
  placeholder,
  min,
  max,
  className,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  min?: number;
  max?: number;
  className?: string;
}) {
  return (
    <input
      type="number"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      min={min}
      max={max}
      className={`rounded-md border border-slate-700 bg-slate-900/60 px-2 py-1.5 text-sm tabular-nums text-slate-100 focus:border-amber-400 focus:outline-none ${
        className ?? "w-full"
      }`}
    />
  );
}
