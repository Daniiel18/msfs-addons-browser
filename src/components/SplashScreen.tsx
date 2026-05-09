import { useMemo } from "react";
import { motion } from "framer-motion";
import { Plane } from "lucide-react";

/**
 * Splash screen — pantalla de carga inicial.
 *
 * Diseño minimal y profesional:
 *   · Logo grande centrado con halo radial (anillo girando muy
 *     lento) — un solo elemento visual focal, sin ilustraciones
 *     cartoony.
 *   · Marca de la app en tipografía grande.
 *   · Tagline rotado entre frases temáticas para que cada apertura
 *     no se sienta exactamente igual.
 *   · Barra de progreso fina con gradiente.
 *   · Indicador de tarea actual (sólo la primera pendiente, no
 *     toda la lista — menos ruido visual).
 *   · Footer con counter y versión.
 *
 * El componente NO inicia trabajo. Sólo refleja el estado que
 * `App.tsx` le pasa. La ventana del splash es undecorated (sin
 * controles de cerrar/minimizar) — por diseño no se puede
 * interactuar mientras carga.
 */
export function SplashScreen({ tasks }: { tasks: SplashTask[] }) {
  const total = tasks.length;
  const done = tasks.filter((t) => t.status === "done").length;
  const errored = tasks.filter((t) => t.status === "error");
  const current = tasks.find((t) => t.status === "pending");
  const pct = total === 0 ? 0 : Math.round((done / total) * 100);

  // Tagline aleatorio — recordado durante el ciclo de vida del
  // splash (sin `useMemo` cambiaría en cada render).
  const tagline = useMemo(() => pickTagline(), []);

  return (
    <div
      data-tauri-drag-region
      className="fixed inset-0 z-50 flex flex-col overflow-hidden bg-slate-950 text-slate-100"
    >
      {/* Fondo: gradiente sutil + dot grid muy tenue para textura */}
      <div
        className="pointer-events-none absolute inset-0 bg-gradient-to-br from-slate-950 via-slate-900 to-slate-950"
        aria-hidden
      />
      <div
        className="pointer-events-none absolute inset-0 opacity-[0.04]"
        aria-hidden
        style={{
          backgroundImage:
            "radial-gradient(rgb(148, 163, 184) 1px, transparent 1px)",
          backgroundSize: "24px 24px",
        }}
      />
      {/* Halo verde difuso detrás del logo, da profundidad */}
      <div
        className="pointer-events-none absolute left-1/2 top-[38%] h-72 w-72 -translate-x-1/2 -translate-y-1/2 rounded-full bg-emerald-500/10 blur-3xl"
        aria-hidden
      />

      <div className="relative flex flex-1 flex-col items-center justify-center px-8 pb-6 pt-2">
        {/* Logo con anillo girando — el único elemento animado
            grande del splash. Suave, profesional. */}
        <div className="relative mb-7 flex h-24 w-24 items-center justify-center">
          {/* Anillo exterior — gira muy despacio en sentido horario */}
          <motion.div
            animate={{ rotate: 360 }}
            transition={{ duration: 8, repeat: Infinity, ease: "linear" }}
            className="absolute inset-0 rounded-full"
            style={{
              background:
                "conic-gradient(from 0deg, transparent 0deg, transparent 270deg, rgb(52, 211, 153) 320deg, rgb(16, 185, 129) 360deg)",
              mask: "radial-gradient(circle, transparent 38px, black 39px, black 47px, transparent 48px)",
              WebkitMask:
                "radial-gradient(circle, transparent 38px, black 39px, black 47px, transparent 48px)",
            }}
          />
          {/* Anillo interior — gira despacio anti-horario */}
          <motion.div
            animate={{ rotate: -360 }}
            transition={{ duration: 14, repeat: Infinity, ease: "linear" }}
            className="absolute inset-2 rounded-full opacity-50"
            style={{
              background:
                "conic-gradient(from 90deg, transparent 0deg, transparent 290deg, rgb(56, 189, 248) 360deg)",
              mask: "radial-gradient(circle, transparent 30px, black 31px, black 36px, transparent 37px)",
              WebkitMask:
                "radial-gradient(circle, transparent 30px, black 31px, black 36px, transparent 37px)",
            }}
          />
          {/* Tarjeta central con el icono */}
          <div className="relative flex h-16 w-16 items-center justify-center rounded-2xl bg-slate-900 ring-1 ring-emerald-500/30 shadow-2xl shadow-emerald-500/20">
            <Plane className="h-8 w-8 text-emerald-300" strokeWidth={1.5} />
          </div>
        </div>

        {/* Marca + tagline */}
        <h1 className="text-center text-xl font-semibold tracking-tight text-slate-50">
          MSFS Addons Browser
        </h1>
        <p className="mt-1.5 text-center text-[11px] tracking-wide text-slate-500">
          {tagline}
        </p>

        {/* Barra de progreso fina */}
        <div className="mt-7 w-full max-w-[300px]">
          <div className="relative h-1 w-full overflow-hidden rounded-full bg-slate-800">
            <motion.div
              className="absolute inset-y-0 left-0 rounded-full bg-gradient-to-r from-emerald-500 via-emerald-400 to-cyan-400"
              initial={{ width: 0 }}
              animate={{ width: `${pct}%` }}
              transition={{ duration: 0.4, ease: "easeOut" }}
            />
            {/* Brillo deslizante encima de la barra */}
            <motion.div
              className="absolute inset-y-0 w-12 bg-gradient-to-r from-transparent via-white/30 to-transparent"
              animate={{ x: ["-50%", "350%"] }}
              transition={{
                duration: 2.2,
                repeat: Infinity,
                ease: "easeInOut",
              }}
            />
          </div>

          {/* Indicador de tarea actual — sólo la activa, no
              toda la lista. Menos ruido. */}
          <div className="mt-3 flex h-4 items-center justify-center text-[11px] text-slate-400">
            {errored.length > 0 ? (
              <span className="text-rose-300">
                {errored.length}{" "}
                {errored.length === 1 ? "error" : "errores"} ·{" "}
                {current?.label ?? "Finalizando"}
              </span>
            ) : current ? (
              <motion.span
                key={current.label}
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                transition={{ duration: 0.2 }}
              >
                {current.label}…
              </motion.span>
            ) : (
              <motion.span
                key="ready"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                className="text-emerald-300"
              >
                Listo
              </motion.span>
            )}
          </div>
        </div>
      </div>

      {/* Footer */}
      <div className="relative flex items-center justify-between border-t border-slate-800/60 bg-slate-950/40 px-5 py-2.5 text-[10px] text-slate-600">
        <span className="font-mono tabular-nums">
          {done} / {total}
        </span>
        <span className="uppercase tracking-widest">v0.1.5</span>
      </div>
    </div>
  );
}

export interface SplashTask {
  label: string;
  status: "pending" | "done" | "error";
  error?: string;
}

const TAGLINES = [
  "Preparando catálogo de addons…",
  "Sincronizando carpeta Community…",
  "Cargando aeropuertos del mundo…",
  "Calentando motores…",
  "Verificando últimas versiones…",
  "Trazando rutas en el mapa…",
];

function pickTagline(): string {
  return TAGLINES[Math.floor(Math.random() * TAGLINES.length)];
}
