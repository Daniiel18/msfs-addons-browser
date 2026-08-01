/**
 * (v7) Medalla de un logro: cinta + disco metálico por nivel + icono propio +
 * pips de nivel. El mismo sistema para los 23 logros — el nivel (bronce →
 * diamante) cambia el metal, así que la medalla evoluciona con el progreso.
 * `tier = null` (aún sin desbloquear) → medalla en gris.
 */

export const TIERS = [
  { key: "bronze", a: "#f3c08a", b: "#a2582c", ring: "#e0a06a" },
  { key: "silver", a: "#f2f7fb", b: "#8593a5", ring: "#cbd5e1" },
  { key: "gold", a: "#ffe9a8", b: "#c9930d", ring: "#fcd34d" },
  { key: "platinum", a: "#eaf6ff", b: "#6f8fa6", ring: "#bae6fd" },
  { key: "diamond", a: "#e9d8ff", b: "#6d3fc4", ring: "#c4b5fd" },
] as const;

const LOCKED = { a: "#5b6675", b: "#333d4c", ring: "#4a5565" };

/** Iconos (trazo, viewBox 24x24) — uno por logro. */
const ICONS: Record<string, string> = {
  plane:
    "M17.8 19.2 16 11l3.5-3.5C21 6 21.5 4 21 3c-1-.5-3 0-4.5 1.5L13 8 4.8 6.2c-.5-.1-.9.1-1.1.5l-.3.5c-.2.5-.1 1 .3 1.3L9 12l-2 3H4l-1 1 3 2 2 3 1-1v-3l3-2 3.5 5.3c.3.4.8.5 1.3.3l.5-.2c.4-.3.6-.7.5-1.2z",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM12 7v5l3.5 2",
  route:
    "M6 16.4a2.6 2.6 0 1 0 0 5.2 2.6 2.6 0 0 0 0-5.2zM18 2.4a2.6 2.6 0 1 0 0 5.2 2.6 2.6 0 0 0 0-5.2zM9 19h6.5a3.5 3.5 0 0 0 0-7h-7a3.5 3.5 0 0 1 0-7H15",
  trend: "M22 7 13.5 15.5 8.5 10.5 2 17M16 7h6v6",
  takeoff:
    "M2 21h20M4.5 15.5 3 12l1.6-.5 2 1.8 3.4-.9-3-5.6L9 6.2l4.6 4.2 4.6-1.2a2 2 0 0 1 1 3.87L5.9 17.2a1.6 1.6 0 0 1-1.4-1.7z",
  landing:
    "M2 21h20M3.2 9.1 4 5.4l1.7.2.9 2.6 3.4.9L9.5 2.6l2.1.6 2.5 5.8 4.6 1.2a2 2 0 0 1-.5 3.94L4.6 12.2a1.6 1.6 0 0 1-1.4-3.1z",
  feather:
    "M20.2 3.8a5.5 5.5 0 0 0-7.8 0L4 12.2V20h7.8l8.4-8.4a5.5 5.5 0 0 0 0-7.8zM16 8 6 18M14 6h4v4",
  sparkle:
    "m12 3-1.9 5.8a2 2 0 0 1-1.3 1.3L3 12l5.8 1.9a2 2 0 0 1 1.3 1.3L12 21l1.9-5.8a2 2 0 0 1 1.3-1.3L21 12l-5.8-1.9a2 2 0 0 1-1.3-1.3z",
  flame:
    "M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.4-.5-2-1-3-1.1-2.1-.2-4 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.2.4-2.3 1-3a2.5 2.5 0 0 0 2.5 2.5z",
  pin: "M20 10c0 6-8 12-8 12s-8-6-8-12a8 8 0 0 1 16 0zM12 7.2a2.8 2.8 0 1 0 0 5.6 2.8 2.8 0 0 0 0-5.6z",
  globe:
    "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM3 12h18M12 3a15 15 0 0 1 4 9 15 15 0 0 1-4 9 15 15 0 0 1-4-9 15 15 0 0 1 4-9z",
  border: "M12 2v20M6 6h12M4 12h16M7 18h10",
  layers: "m12 2 9 5-9 5-9-5 9-5zm-9 10 9 5 9-5m-18 5 9 5 9-5",
  wrench:
    "M15 5a5 5 0 0 0-6.6 6.6L3 17v4h4l5.4-5.4A5 5 0 0 0 19 9l-3 3-3-3 3-3a5 5 0 0 0-1-1z",
  alt: "M12 21V5m-6 6 6-6 6 6M4 3h16",
  swap: "M7 4 3 8l4 4M3 8h13m1 12 4-4-4-4m4 4H8",
  building:
    "M4 21V6l7-3v18M11 9h7a2 2 0 0 1 2 2v10M7 9h.01M7 13h.01M7 17h.01M15 13h.01M15 17h.01",
  award: "M12 3a6 6 0 1 0 0 12 6 6 0 0 0 0-12zm3.5 10.5 1.5 8-5-3-5 3 1.5-8",
  star: "m12 3 2.7 5.9 6.3.7-4.7 4.3 1.3 6.3L12 17.8 6.4 20.2l1.3-6.3L3 9.6l6.3-.7z",
  check: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zm-3.5 9.2 2.6 2.6 4.6-5.2",
  moon: "M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9z",
  users:
    "M16 20v-2a4 4 0 0 0-4-4H7a4 4 0 0 0-4 4v2M9.5 3.6a3.4 3.4 0 1 0 0 6.8 3.4 3.4 0 0 0 0-6.8zM21 20v-2a4 4 0 0 0-3-3.87M16.5 3.6a4 4 0 0 1 0 6.8",
  trophy:
    "M7 4h10v5a5 5 0 0 1-10 0zM7 6H5a2.2 2.2 0 0 0 0 4.4h1.2M17 6h2a2.2 2.2 0 0 1 0 4.4h-1.2M12 14v3M8.5 21h7M10 21c0-2 2-2 2-4M14 21c0-2-2-2-2-4",
};

export function MedalBadge({
  icon,
  tier,
  size = 62,
}: {
  icon: string;
  /** Índice del nivel alcanzado; null = bloqueado (gris). */
  tier: number | null;
  size?: number;
}) {
  const locked = tier == null;
  const idx = Math.min(Math.max(tier ?? 0, 0), TIERS.length - 1);
  const C = locked ? LOCKED : TIERS[idx];
  // Ids únicos por instancia para que los degradados no colisionen.
  const uid = `${icon}-${locked ? "x" : idx}`;
  const n = idx + 1;
  const x0 = 50 - ((n - 1) * 5.5) / 2;
  const path = ICONS[icon] ?? ICONS.trophy;

  return (
    <svg
      viewBox="0 0 100 112"
      width={size}
      height={size * 1.12}
      className="shrink-0"
      aria-hidden
    >
      <defs>
        <linearGradient id={`m-${uid}`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor={C.a} />
          <stop offset="0.5" stopColor={C.ring} />
          <stop offset="1" stopColor={C.b} />
        </linearGradient>
        <linearGradient id={`r-${uid}`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor={locked ? "#2b3441" : "#334155"} />
          <stop offset="1" stopColor={locked ? "#1c2430" : "#1e293b"} />
        </linearGradient>
      </defs>
      {/* cintas */}
      <path d="M31 2h13l12 34-14 8z" fill={`url(#r-${uid})`} />
      <path d="M69 2H56L44 36l14 8z" fill={`url(#r-${uid})`} opacity="0.72" />
      {/* disco */}
      <circle cx="50" cy="66" r="31" fill={`url(#m-${uid})`} />
      <circle
        cx="50"
        cy="66"
        r="31"
        fill="none"
        stroke="rgba(0,0,0,.35)"
        strokeWidth="1.4"
      />
      <circle
        cx="50"
        cy="66"
        r="28.6"
        fill="none"
        stroke={locked ? "#5b6675" : C.a}
        strokeWidth="1.1"
        strokeDasharray="1.6 4.2"
        strokeLinecap="round"
        opacity="0.65"
      />
      <circle
        cx="50"
        cy="66"
        r="25.5"
        fill={locked ? "#141b26" : "#101827"}
        opacity="0.92"
      />
      <circle
        cx="50"
        cy="66"
        r="25.5"
        fill="none"
        stroke={C.ring}
        strokeWidth="1"
        opacity="0.55"
      />
      {/* brillo */}
      {!locked && (
        <path
          d="M50 35a31 31 0 0 0-21.9 52.9A31 31 0 0 1 50 35z"
          fill="#fff"
          opacity="0.16"
        />
      )}
      {/* icono */}
      <g
        transform="translate(50 66) scale(.95) translate(-12 -12)"
        fill="none"
        stroke={locked ? "#6b7684" : C.ring}
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d={path} />
      </g>
      {/* pips de nivel */}
      {Array.from({ length: n }, (_, i) => (
        <circle
          key={i}
          cx={x0 + i * 5.5}
          cy="104"
          r="2.1"
          fill={locked ? "#3b4553" : TIERS[i].ring}
        />
      ))}
    </svg>
  );
}
