import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { Search, X } from "lucide-react";
import type { TechLogAircraft, TechLogDefect, TechLogPart } from "../lib/types";
import { t } from "../lib/i18n";
import { cn } from "../lib/cn";

/**
 * (26/10) Etapa 6 — Vista de estado del Tech Log (Hangar → Mantenimiento).
 *
 * Misma idea que la página "Status" de Fenix TechLog, con dibujos PROPIOS
 * (SVG generado aquí, nada copiado de TechLog): el avión visto desde arriba
 * con sus motores, APU y tren y marcas donde hay fallos; cada motor en corte
 * por secciones con su margen EGT; ruedas y frenos; y los sistemas por
 * capítulo ATA, un punto por pieza. Las acciones (reparar, reemplazar) son las
 * mismas del Hangar: las pasa `renderActions`.
 */

type Tone = "ok" | "worn" | "degraded" | "failed" | "none";

const TONE_FILL: Record<Tone, string> = {
  ok: "#2f8f5f",
  worn: "#a8b934",
  degraded: "#f0a020",
  failed: "#e5484d",
  none: "#334155",
};
const TONE_TEXT: Record<Tone, string> = {
  ok: "text-emerald-300",
  worn: "text-lime-300",
  degraded: "text-amber-300",
  failed: "text-rose-300",
  none: "text-sf-muted",
};
const TONE_RANK: Record<Tone, number> = { none: -1, ok: 0, worn: 1, degraded: 2, failed: 3 };

function partTone(p: TechLogPart): Tone {
  if (p.status === "Failed") return "failed";
  if (p.status === "Degraded") return "degraded";
  return p.worn ? "worn" : "ok";
}

function worst(tones: Tone[]): Tone {
  return tones.reduce<Tone>((a, b) => (TONE_RANK[b] > TONE_RANK[a] ? b : a), "none");
}

function defectTone(d: TechLogDefect): Tone {
  return d.status === "Open" && d.category === "NoGo" ? "failed" : d.status === "Open" ? "degraded" : "worn";
}

const pct = (v: number) => `${Math.round(v)}%`;
const num = (v: number, d = 0) =>
  v.toLocaleString(undefined, { minimumFractionDigits: d, maximumFractionDigits: d });

// ---------------------------------------------------------------------------
// Clasificación de piezas
// ---------------------------------------------------------------------------

/** Motor de una pieza (1, 2…) o `null`. */
function engineOf(p: TechLogPart): number | null {
  const fromName = /\b(?:engine|eng|motor)\s*(\d)/i.exec(p.name)?.[1];
  const engineAta = p.ata >= 70 && p.ata <= 80;
  const fromPos = /^(\d)/.exec(p.position ?? "")?.[1];
  if (engineAta || /^ENG_/.test(p.key) || (p.ata === 26 && /FIRE_LOOP|ENG/.test(p.key))) {
    const n = Number(fromName ?? fromPos);
    return Number.isFinite(n) && n > 0 ? n : null;
  }
  return fromName ? Number(fromName) : null;
}

type Section = "fan" | "comp" | "comb" | "vanes" | "hpt" | "lpt";
const SECTIONS: Section[] = ["fan", "comp", "comb", "vanes", "hpt", "lpt"];

function engineSection(p: TechLogPart): Section | null {
  const byKey: Record<string, Section> = {
    ENG_FAN: "fan",
    ENG_HPC: "comp",
    ENG_LPC: "comp",
    ENG_CORE: "comp",
    ENG_COMBUSTOR: "comb",
    IGNITER: "comb",
    ENG_NGV: "vanes",
    ENG_HPT: "hpt",
    ENG_LPT: "lpt",
  };
  if (byKey[p.key]) return byKey[p.key];
  const n = p.name.toLowerCase();
  if (/\bfan\b/.test(n)) return "fan";
  if (/compressor|\bhpc\b|\blpc\b|booster|core/.test(n)) return "comp";
  if (/combust|burner|ignit/.test(n)) return "comb";
  if (/vane|\bngv\b|nozzle guide/.test(n)) return "vanes";
  if (/hp turbine|\bhpt\b|high.pressure turbine/.test(n)) return "hpt";
  if (/lp turbine|\blpt\b|low.pressure turbine/.test(n)) return "lpt";
  return null;
}

const isTyre = (p: TechLogPart) => /TYRE|TIRE/.test(p.key) || /\b(tyre|tire)\b/i.test(p.name);
const isNoseTyre = (p: TechLogPart) =>
  /NOSE/.test(p.key) || /nose/i.test(p.name) || /^N/i.test(p.position ?? "");
const isBrake = (p: TechLogPart) =>
  p.key === "BRAKE" || (/\bbrake\b/i.test(p.name) && !/temp|control|unit|valve|accum|bscu|abcu|auto/i.test(p.name));

function tyreLabel(p: TechLogPart): string {
  const pos = (p.position ?? "").toUpperCase();
  if (isNoseTyre(p)) return pos.startsWith("N") ? pos : `N${pos || "1"}`;
  return pos || "?";
}

// ---------------------------------------------------------------------------
// Sistemas por capítulo ATA
// ---------------------------------------------------------------------------

const SYSTEMS: { code: string; atas: number[] }[] = [
  { code: "COND", atas: [21] },
  { code: "AUTO FLT", atas: [22] },
  { code: "COMMS", atas: [23] },
  { code: "ELEC", atas: [24] },
  { code: "EQUIP", atas: [25] },
  { code: "FIRE", atas: [26] },
  { code: "F/CTL", atas: [27] },
  { code: "FUEL", atas: [28] },
  { code: "HYD", atas: [29] },
  { code: "ANTI ICE", atas: [30] },
  { code: "DISPLAYS", atas: [31] },
  { code: "WHEEL", atas: [32] },
  { code: "LIGHTS", atas: [33] },
  { code: "NAV", atas: [34] },
  { code: "OXY · WATER", atas: [35, 38] },
  { code: "BLEED", atas: [36] },
  { code: "APU", atas: [49] },
  { code: "DOOR", atas: [52] },
  { code: "WINDOWS", atas: [56] },
];

interface SystemGroup {
  id: string;
  code: string;
  ata: string;
  label: string;
  parts: TechLogPart[];
  tone: Tone;
}

function groupSystems(parts: TechLogPart[]): SystemGroup[] {
  const groups = new Map<string, SystemGroup>();
  const add = (id: string, code: string, ata: string, label: string, p: TechLogPart) => {
    const g = groups.get(id) ?? { id, code, ata, label, parts: [], tone: "none" as Tone };
    g.parts.push(p);
    groups.set(id, g);
  };
  for (const p of parts) {
    const eng = engineOf(p);
    if (eng && p.ata >= 70) {
      add(`eng${eng}`, `ENG ${eng}`, "ATA 70-80", t("tlv.sys.engine", { n: String(eng) }), p);
      continue;
    }
    const sys = SYSTEMS.find((s) => s.atas.includes(p.ata));
    if (sys) add(sys.code, sys.code, `ATA ${sys.atas.join("/")}`, t(`tlv.sys.${sys.atas[0]}`), p);
    else add(`ata${p.ata}`, `ATA ${p.ata}`, `ATA ${p.ata}`, p.system || `ATA ${p.ata}`, p);
  }
  const order = (g: SystemGroup) => {
    if (g.id.startsWith("eng")) return 1000 + Number(g.id.slice(3));
    const i = SYSTEMS.findIndex((s) => s.code === g.id);
    return i >= 0 ? i : 500 + Number(g.id.replace(/\D/g, ""));
  };
  return [...groups.values()]
    .map((g) => ({
      ...g,
      parts: [...g.parts].sort((a, b) => TONE_RANK[partTone(b)] - TONE_RANK[partTone(a)] || a.remainingPct - b.remainingPct),
      tone: worst(g.parts.map(partTone)),
    }))
    .sort((a, b) => order(a) - order(b));
}

// ---------------------------------------------------------------------------
// Siluetas (dibujo propio, paramétrico por familia)
// ---------------------------------------------------------------------------

export type Silhouette = "narrow" | "b737" | "wide" | "quad" | "regional" | "turboprop";

export function silhouetteFor(variant: string, model?: string | null): Silhouette {
  const s = `${variant} ${model ?? ""}`.toUpperCase();
  if (/A380|747|B74\d|A340|A34\d/.test(s)) return "quad";
  if (/\bCRJ|ERJ.?1[34]|EMB.?1[34]5|FOKKER|F100|\bMD.?[89]\d|\bDC.?9|\b717\b|CHALLENGER|GULFSTREAM|LEARJET|CITATION|PHENOM/.test(s))
    return "regional";
  if (/\bATR\b|ATR.?72|ATR.?42|DASH.?8|Q400|\bDHC|TURBOPROP|KING ?AIR|BEECH|CESSNA|C208|CARAVAN|SAAB|JETSTREAM|PC.?12|TBM|C172|C152/.test(s))
    return "turboprop";
  if (/777|787|A350|A330|A300|A310|767|A35\d|A33\d|B77|B78|B76|AWIDE/.test(s)) return "wide";
  if (/737|B73\d|B3[89]M|\bMAX\b/.test(s)) return "b737";
  return "narrow";
}

interface Shape {
  w: number; // ancho del fuselaje
  top: number;
  bottom: number;
  nose: number; // largo del morro
  tailStart: number; // donde empieza a estrecharse la cola
  wing: { rootLead: number; rootTrail: number; span: number; tipLead: number; tipTrail: number };
  stab: { rootLead: number; rootTrail: number; span: number; tipLead: number; tipTrail: number };
  engines: { x: number; y: number; w: number; h: number; prop?: boolean }[]; // x relativo al eje, a la izquierda (negativo)
  noseGear: number;
  mainGear: { x: number; y: number };
  apuY: number;
}

const CX = 200;

function shapeOf(kind: Silhouette): Shape {
  switch (kind) {
    case "b737":
      return {
        w: 30, top: 34, bottom: 404, nose: 46, tailStart: 330,
        wing: { rootLead: 182, rootTrail: 250, span: 158, tipLead: 268, tipTrail: 288 },
        stab: { rootLead: 350, rootTrail: 388, span: 66, tipLead: 384, tipTrail: 398 },
        engines: [{ x: -58, y: 162, w: 24, h: 52 }],
        noseGear: 92, mainGear: { x: 36, y: 252 }, apuY: 396,
      };
    case "wide":
      return {
        w: 42, top: 14, bottom: 428, nose: 60, tailStart: 340,
        wing: { rootLead: 170, rootTrail: 262, span: 186, tipLead: 290, tipTrail: 306 },
        stab: { rootLead: 352, rootTrail: 398, span: 80, tipLead: 394, tipTrail: 410 },
        engines: [{ x: -76, y: 150, w: 32, h: 66 }],
        noseGear: 84, mainGear: { x: 34, y: 262 }, apuY: 420,
      };
    case "quad":
      return {
        w: 44, top: 12, bottom: 430, nose: 64, tailStart: 342,
        wing: { rootLead: 168, rootTrail: 262, span: 188, tipLead: 296, tipTrail: 312 },
        stab: { rootLead: 354, rootTrail: 400, span: 82, tipLead: 396, tipTrail: 412 },
        engines: [
          { x: -66, y: 152, w: 26, h: 56 },
          { x: -128, y: 196, w: 24, h: 52 },
        ],
        noseGear: 84, mainGear: { x: 32, y: 262 }, apuY: 422,
      };
    case "regional":
      return {
        w: 26, top: 40, bottom: 404, nose: 44, tailStart: 318,
        wing: { rootLead: 196, rootTrail: 250, span: 140, tipLead: 262, tipTrail: 278 },
        stab: { rootLead: 378, rootTrail: 404, span: 62, tipLead: 400, tipTrail: 412 },
        engines: [{ x: -30, y: 300, w: 18, h: 52 }],
        noseGear: 88, mainGear: { x: 30, y: 248 }, apuY: 396,
      };
    case "turboprop":
      return {
        w: 26, top: 44, bottom: 400, nose: 40, tailStart: 320,
        wing: { rootLead: 170, rootTrail: 214, span: 176, tipLead: 176, tipTrail: 206 },
        stab: { rootLead: 362, rootTrail: 390, span: 64, tipLead: 372, tipTrail: 390 },
        engines: [{ x: -62, y: 150, w: 18, h: 62, prop: true }],
        noseGear: 90, mainGear: { x: 62, y: 214 }, apuY: 392,
      };
    default:
      return {
        w: 30, top: 24, bottom: 414, nose: 50, tailStart: 334,
        wing: { rootLead: 184, rootTrail: 254, span: 166, tipLead: 272, tipTrail: 290 },
        stab: { rootLead: 356, rootTrail: 394, span: 70, tipLead: 390, tipTrail: 404 },
        engines: [{ x: -70, y: 160, w: 26, h: 56 }],
        noseGear: 96, mainGear: { x: 40, y: 256 }, apuY: 404,
      };
  }
}

/** Motores del dibujo, numerados de izquierda a derecha (1 = el más a la izquierda). */
function enginesOf(s: Shape) {
  const left = [...s.engines].sort((a, b) => a.x - b.x); // el más exterior primero
  const all = [
    ...left.map((e) => ({ ...e, x: CX + e.x })),
    ...[...left].reverse().map((e) => ({ ...e, x: CX - e.x })),
  ];
  return all.map((e, i) => ({ ...e, n: i + 1 }));
}

function fuselagePath(s: Shape): string {
  const l = CX - s.w / 2;
  const r = CX + s.w / 2;
  return [
    `M ${l} ${s.top + s.nose}`,
    `C ${l} ${s.top + s.nose * 0.35} ${CX - s.w * 0.22} ${s.top} ${CX} ${s.top}`,
    `C ${CX + s.w * 0.22} ${s.top} ${r} ${s.top + s.nose * 0.35} ${r} ${s.top + s.nose}`,
    `L ${r} ${s.tailStart}`,
    `L ${CX + s.w * 0.1} ${s.bottom}`,
    `L ${CX - s.w * 0.1} ${s.bottom}`,
    `L ${l} ${s.tailStart} Z`,
  ].join(" ");
}

function surface(rootX: number, d: Shape["wing"], side: 1 | -1): string {
  const tip = CX + side * d.span;
  return `M ${rootX} ${d.rootLead} L ${tip} ${d.tipLead} L ${tip} ${d.tipTrail} L ${rootX} ${d.rootTrail} Z`;
}

/** Dónde se marca un fallo según su capítulo ATA. */
function anchorFor(ata: number, s: Shape): { x: number; y: number } {
  const wingMid = (side: 1 | -1, f: number, lead = true) => {
    const span = s.wing.span * f;
    const y = lead
      ? s.wing.rootLead + (s.wing.tipLead - s.wing.rootLead) * f
      : s.wing.rootTrail + (s.wing.tipTrail - s.wing.rootTrail) * f;
    return { x: CX + side * span, y: y + (lead ? 6 : -6) };
  };
  switch (ata) {
    case 22: case 31: case 35: case 56:
      return { x: CX, y: s.top + s.nose * 0.75 };
    case 24: case 34:
      return { x: CX, y: s.top + s.nose * 1.35 };
    case 23:
      return { x: CX, y: s.top + s.nose * 2 };
    case 52:
      return { x: CX - s.w / 2, y: s.top + s.nose * 1.6 };
    case 25:
      return { x: CX, y: (s.top + s.nose + s.wing.rootLead) / 2 + 10 };
    case 21:
      return { x: CX, y: s.wing.rootLead - 8 };
    case 29: case 32:
      return { x: CX, y: (s.wing.rootLead + s.wing.rootTrail) / 2 };
    case 28:
      return wingMid(-1, 0.45, false);
    case 30:
      return wingMid(-1, 0.7, true);
    case 27:
      return wingMid(1, 0.6, false);
    case 33:
      return { x: CX + s.wing.span - 6, y: s.wing.tipLead + 6 };
    case 36:
      return wingMid(1, 0.25, true);
    case 26: case 38:
      return { x: CX, y: s.tailStart - 20 };
    case 49:
      return { x: CX, y: s.apuY - 22 };
    default:
      return { x: CX, y: (s.wing.rootTrail + s.tailStart) / 2 };
  }
}

interface Mark {
  key: string;
  x: number;
  y: number;
  tone: Tone;
  items: string[];
  system: string | null;
}

function AircraftDiagram({
  kind,
  data,
  parts,
  onEngine,
  onSystem,
  engineSel,
}: {
  kind: Silhouette;
  data: TechLogAircraft;
  parts: TechLogPart[];
  onEngine: (n: number) => void;
  onSystem: (id: string) => void;
  engineSel: number | null;
}) {
  const s = shapeOf(kind);
  const engines = enginesOf(s);
  const engTone = (n: number) => {
    const ps = parts.filter((p) => engineOf(p) === n);
    return ps.length ? worst(ps.map(partTone)) : "none";
  };
  const tyres = parts.filter(isTyre);
  const brakes = parts.filter(isBrake);
  const gearTone = (side: "nose" | "left" | "right") => {
    const nose = tyres.filter(isNoseTyre);
    const mains = tyres.filter((p) => !isNoseTyre(p));
    const half = Math.ceil(mains.length / 2);
    const sorted = [...mains].sort((a, b) => tyreLabel(a).localeCompare(tyreLabel(b), undefined, { numeric: true }));
    const bk = [...brakes].sort((a, b) => (a.position ?? "").localeCompare(b.position ?? "", undefined, { numeric: true }));
    const bHalf = Math.ceil(bk.length / 2);
    const ps =
      side === "nose" ? nose : side === "left" ? [...sorted.slice(0, half), ...bk.slice(0, bHalf)] : [...sorted.slice(half), ...bk.slice(bHalf)];
    return ps.length ? worst(ps.map(partTone)) : "none";
  };
  const apuParts = parts.filter((p) => p.ata === 49);
  const apuTone = apuParts.length ? worst(apuParts.map(partTone)) : "none";

  // Marcas de fallo: defectos abiertos o diferidos y piezas con problema que
  // no se ven ya en un motor o en el tren.
  const marks = useMemo(() => {
    const byAnchor = new Map<string, Mark>();
    const push = (ata: number, tone: Tone, text: string, system: string | null) => {
      if (ata <= 12) return; // inspecciones y servicio: no tienen sitio en el avión
      const a = anchorFor(ata, s);
      const key = `${Math.round(a.x)}:${Math.round(a.y)}`;
      const m = byAnchor.get(key) ?? { key, x: a.x, y: a.y, tone: "none" as Tone, items: [], system };
      m.items.push(text);
      m.tone = worst([m.tone, tone]);
      byAnchor.set(key, m);
    };
    for (const d of data.defects) {
      const n = /\b(?:engine|eng)\s*(\d)/i.exec(`${d.title} ${d.position ?? ""}`)?.[1];
      if (d.ata >= 70 && n) continue; // se ve en su motor
      push(d.ata, defectTone(d), d.title, SYSTEMS.find((x) => x.atas.includes(d.ata))?.code ?? null);
    }
    for (const p of parts) {
      if (partTone(p) === "ok" || engineOf(p) || isTyre(p) || isBrake(p) || p.ata === 49) continue;
      push(p.ata, partTone(p), `${p.name} · ${pct(p.remainingPct)}`, SYSTEMS.find((x) => x.atas.includes(p.ata))?.code ?? null);
    }
    return [...byAnchor.values()];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [data.defects, parts, kind]);

  const gearRects = (x: number, y: number, tone: Tone, key: string) => (
    <g key={key}>
      <rect x={x - 5} y={y - 6} width={4} height={12} rx={1} fill={TONE_FILL[tone === "none" ? "ok" : tone]} />
      <rect x={x + 1} y={y - 6} width={4} height={12} rx={1} fill={TONE_FILL[tone === "none" ? "ok" : tone]} />
    </g>
  );

  const pill = (x: number, y: number, label: string, tone: Tone, active: boolean, onClick: () => void, key: string) => {
    const w = label.length * 7 + 26;
    return (
      <g key={key} onClick={onClick} className="cursor-pointer">
        <rect
          x={x - w / 2}
          y={y - 9}
          width={w}
          height={18}
          rx={9}
          fill="#0f172a"
          stroke={active ? "#e2e8f0" : "#475569"}
          strokeWidth={active ? 1.6 : 1}
        />
        <circle cx={x - w / 2 + 10} cy={y} r={3.5} fill={TONE_FILL[tone === "none" ? "ok" : tone]} />
        <text x={x - w / 2 + 18} y={y + 3.5} fontSize={10} fontFamily="ui-monospace, monospace" fontWeight={600} fill="#e2e8f0">
          {label}
        </text>
      </g>
    );
  };

  return (
    <svg viewBox="0 0 400 440" className="mx-auto h-auto w-full max-w-[420px]" role="img" aria-label={t("tlv.aircraft")}>
      {/* Alas y estabilizadores */}
      {([-1, 1] as const).map((side) => (
        <g key={side} fill="#1e2533" stroke="#56627a" strokeWidth={1.2} strokeLinejoin="round">
          <path d={surface(CX + (side * s.w) / 2, s.wing, side)} />
          <path d={surface(CX + (side * s.w) / 2 * 0.8, s.stab, side)} />
        </g>
      ))}
      {/* Motores */}
      {engines.map((e) => (
        <g key={e.n}>
          <rect
            x={e.x - e.w / 2}
            y={e.y}
            width={e.w}
            height={e.h}
            rx={e.w / 2.4}
            fill="#1e2533"
            stroke={engineSel === e.n ? "#e2e8f0" : "#56627a"}
            strokeWidth={1.2}
          />
          {e.prop && <line x1={e.x - 22} x2={e.x + 22} y1={e.y - 3} y2={e.y - 3} stroke="#56627a" strokeWidth={2} />}
        </g>
      ))}
      {/* Fuselaje */}
      <path d={fuselagePath(s)} fill="#151b26" stroke="#56627a" strokeWidth={1.4} />
      {/* Deriva (vista desde arriba: una línea) */}
      <line x1={CX} x2={CX} y1={s.stab.rootLead - 24} y2={s.bottom - 6} stroke="#56627a" strokeWidth={2.5} strokeLinecap="round" />
      {/* Cabina */}
      <path
        d={`M ${CX - s.w * 0.28} ${s.top + s.nose * 0.55} Q ${CX} ${s.top + s.nose * 0.3} ${CX + s.w * 0.28} ${s.top + s.nose * 0.55} L ${CX + s.w * 0.24} ${s.top + s.nose * 0.68} Q ${CX} ${s.top + s.nose * 0.55} ${CX - s.w * 0.24} ${s.top + s.nose * 0.68} Z`}
        fill="#2a4a6b"
      />
      {/* Tren */}
      {gearRects(CX, s.noseGear, gearTone("nose"), "gn")}
      {gearRects(CX - s.mainGear.x, s.mainGear.y, gearTone("left"), "gl")}
      {gearRects(CX + s.mainGear.x, s.mainGear.y, gearTone("right"), "gr")}
      {/* Marcas de fallo */}
      {marks.map((m) => (
        <g key={m.key} className="cursor-pointer" onClick={() => m.system && onSystem(m.system)}>
          <title>{m.items.join("\n")}</title>
          <circle cx={m.x} cy={m.y} r={11} fill={TONE_FILL[m.tone]} opacity={0.22}>
            <animate attributeName="r" values="9;13;9" dur="2s" repeatCount="indefinite" />
          </circle>
          <circle cx={m.x} cy={m.y} r={6.5} fill={TONE_FILL[m.tone]} stroke="#0f172a" strokeWidth={1.2} />
          {m.items.length > 1 && (
            <text x={m.x} y={m.y + 3} textAnchor="middle" fontSize={8} fontWeight={700} fill="#0f172a">
              {m.items.length}
            </text>
          )}
        </g>
      ))}
      {/* Etiquetas de motores y APU */}
      {engines.map((e) =>
        pill(
          e.x + (e.x < CX ? -6 : 6),
          e.y - 16,
          `ENG ${e.n}`,
          engTone(e.n),
          engineSel === e.n,
          () => onEngine(e.n),
          `p${e.n}`,
        ),
      )}
      {pill(CX, s.apuY, "APU", apuTone, false, () => onSystem("APU"), "apu")}
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Motores
// ---------------------------------------------------------------------------

function EgtGauge({ value }: { value: number | undefined }) {
  const r = 26;
  const c = 2 * Math.PI * r;
  const arc = c * 0.75;
  const frac = value == null ? 0 : Math.max(0, Math.min(1, value / 90));
  const color = value == null ? "#475569" : value > 25 ? "#2f8f5f" : value > 10 ? "#f0a020" : "#e5484d";
  return (
    <svg viewBox="0 0 70 64" className="h-16 w-[70px] shrink-0">
      <g transform="rotate(135 35 34)">
        <circle cx={35} cy={34} r={r} fill="none" stroke="#273142" strokeWidth={6} strokeDasharray={`${arc} ${c}`} strokeLinecap="round" />
        <circle
          cx={35}
          cy={34}
          r={r}
          fill="none"
          stroke={color}
          strokeWidth={6}
          strokeDasharray={`${arc * frac} ${c}`}
          strokeLinecap="round"
        />
      </g>
      <text x={35} y={36} textAnchor="middle" fontSize={13} fontWeight={700} fill="#e2e8f0">
        {value == null ? "—" : `${Math.round(value)}°`}
      </text>
      <text x={35} y={48} textAnchor="middle" fontSize={7} fill="#94a3b8">
        {t("tlv.egt_margin")}
      </text>
    </svg>
  );
}

/** Corte del motor: fan, compresor, cámara, álabes guía, turbina HP y LP. */
const SECTION_GEOM: Record<Section, { path: string; cx: number }> = {
  fan: { path: "M 14 22 L 52 30 L 52 98 L 14 106 Q 10 64 14 22 Z", cx: 33 },
  comp: { path: "M 62 38 L 150 40 L 150 88 L 62 90 Z", cx: 106 },
  comb: { path: "M 160 44 L 214 44 L 214 84 L 160 84 Z", cx: 187 },
  vanes: { path: "M 224 48 L 262 48 L 262 80 L 224 80 Z", cx: 243 },
  hpt: { path: "M 272 46 L 324 46 L 324 82 L 272 82 Z", cx: 298 },
  lpt: { path: "M 334 44 L 452 56 L 452 72 L 334 84 Z", cx: 393 },
};

function EngineCard({
  n,
  parts,
  egt,
  hours,
  cycles,
  starts,
  selected,
  onSelect,
}: {
  n: number;
  parts: TechLogPart[];
  egt: number | undefined;
  hours: number | undefined;
  cycles: number | undefined;
  starts: number | undefined;
  selected: boolean;
  onSelect: () => void;
}) {
  const bySection = (sec: Section) => parts.filter((p) => engineSection(p) === sec);
  const tone = parts.length ? worst(parts.map(partTone)) : "none";
  const failed = parts.filter((p) => partTone(p) === "failed").length;
  const bad = parts.filter((p) => partTone(p) !== "ok").length;
  const core = parts.filter((p) => engineSection(p));
  const coreMin = core.length ? Math.min(...core.map((p) => p.remainingPct)) : null;
  const worstPart = [...parts].sort((a, b) => a.remainingPct - b.remainingPct)[0];
  return (
    <div
      onClick={onSelect}
      className={cn(
        "cursor-pointer rounded-xl border bg-sf-bg/40 p-3 transition-colors",
        selected ? "border-slate-300/70" : "border-sf-border hover:border-slate-500/60",
      )}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-sm font-semibold text-sf-text">{t("tlv.engine", { n: String(n) })}</span>
        <span
          className={cn(
            "rounded px-1.5 py-0.5 text-[10px] font-medium",
            tone === "failed" ? "bg-rose-500/20 text-rose-300" : bad ? "bg-amber-500/15 text-amber-300" : "bg-emerald-500/15 text-emerald-300",
          )}
        >
          {failed
            ? t(failed === 1 ? "tlv.parts_failed_one" : "tlv.parts_failed", { n: String(failed) })
            : bad
              ? t(bad === 1 ? "tlv.parts_worn_one" : "tlv.parts_worn", { n: String(bad) })
              : t("tlv.parts_good")}
        </span>
      </div>
      <div className="mt-1 flex items-center gap-4">
        <EgtGauge value={egt} />
        <dl className="grid grid-cols-3 gap-x-4 text-[11px]">
          <dt className="text-sf-muted">{t("tlv.hours")}</dt>
          <dt className="text-sf-muted">{t("tlv.cycles")}</dt>
          <dt className="text-sf-muted">{t("tlv.starts")}</dt>
          <dd className="font-mono text-sf-text">{hours != null ? num(hours, 1) : "—"}</dd>
          <dd className="font-mono text-sf-text">{cycles != null ? num(cycles) : "—"}</dd>
          <dd className="font-mono text-sf-text">{starts != null ? num(starts) : "—"}</dd>
        </dl>
      </div>
      <svg viewBox="0 0 470 128" className="mt-1 h-auto w-full">
        <path
          d="M 8 18 Q 4 64 8 110 L 300 104 Q 420 92 462 70 L 462 58 Q 420 36 300 24 Z"
          fill="#151b26"
          stroke="#56627a"
          strokeWidth={1.4}
        />
        {SECTIONS.map((sec) => {
          const ps = bySection(sec);
          const tn: Tone = ps.length ? worst(ps.map(partTone)) : "none";
          return (
            <path key={sec} d={SECTION_GEOM[sec].path} fill={TONE_FILL[tn]} stroke="#0f172a" strokeWidth={1}>
              <title>
                {[t(`tlv.sec.${sec}`), ...ps.map((p) => `${p.name} · ${pct(p.remainingPct)}`)].join("\n")}
              </title>
            </path>
          );
        })}
      </svg>
      <div className="relative h-4 text-[10px] text-sf-muted">
        {SECTIONS.map((sec) => (
          <span
            key={sec}
            className="absolute -translate-x-1/2 whitespace-nowrap underline decoration-dotted underline-offset-2"
            style={{ left: `${(SECTION_GEOM[sec].cx / 470) * 100}%` }}
            title={bySection(sec).map((p) => `${p.name} · ${pct(p.remainingPct)}`).join("\n") || t("tlv.no_parts")}
          >
            {t(`tlv.sec_short.${sec}`)}
          </span>
        ))}
      </div>
      {worstPart && (
        <p className="mt-2 truncate text-[11px] text-sf-muted">
          {worstPart.remainingPct < 99.5 || worstPart.status !== "Ok" ? (
            <>
              {t("tlv.most_worn")}{" "}
              <span className={TONE_TEXT[partTone(worstPart)]}>
                {worstPart.name} · {pct(worstPart.remainingPct)}
              </span>
            </>
          ) : (
            t("tlv.no_wear")
          )}
        </p>
      )}
      {coreMin != null && (
        <div className="mt-2">
          <div className="flex justify-between text-[10px] text-sf-muted">
            <span>{t("tlv.core_life")}</span>
            <span className="tabular-nums">{pct(coreMin)}</span>
          </div>
          <div className="mt-0.5 h-1.5 overflow-hidden rounded-full bg-sf-raised">
            <div
              className="h-full rounded-full"
              style={{ width: `${Math.max(2, coreMin)}%`, backgroundColor: TONE_FILL[coreMin >= 25 ? "ok" : coreMin >= 10 ? "worn" : "failed"] }}
            />
          </div>
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Ruedas y frenos
// ---------------------------------------------------------------------------

function Tyre({ p, brake }: { p: TechLogPart; brake?: TechLogPart }) {
  const tn = partTone(p);
  return (
    <div className="flex w-14 flex-col items-center gap-0.5" title={`${p.name} · ${pct(p.remainingPct)}`}>
      <svg viewBox="0 0 28 42" className="h-11 w-7">
        <defs>
          <pattern id={`tread-${p.id}`} width="28" height="5" patternUnits="userSpaceOnUse">
            <rect width="28" height="5" fill={TONE_FILL[tn]} />
            <line x1="0" y1="4.5" x2="28" y2="4.5" stroke="#0f172a" strokeOpacity={0.35} strokeWidth={1} />
          </pattern>
        </defs>
        <rect x={1} y={1} width={26} height={40} rx={6} fill={`url(#tread-${p.id})`} stroke="#0f172a" strokeOpacity={0.5} />
        <text x={14} y={25} textAnchor="middle" fontSize={9} fontWeight={700} fill="#f8fafc">
          {tyreLabel(p)}
        </text>
      </svg>
      <span className={cn("text-[10px] tabular-nums", TONE_TEXT[tn])}>{pct(p.remainingPct)}</span>
      {brake && (
        <div className="w-full" title={`${brake.name} · ${pct(brake.remainingPct)}`}>
          <div className="h-1 overflow-hidden rounded-full bg-sf-raised">
            <div
              className="h-full rounded-full"
              style={{ width: `${Math.max(2, brake.remainingPct)}%`, backgroundColor: TONE_FILL[partTone(brake)] }}
            />
          </div>
          <div className={cn("text-center text-[9px] tabular-nums", TONE_TEXT[partTone(brake)])}>
            {t("tlv.brake", { p: pct(brake.remainingPct) })}
          </div>
        </div>
      )}
    </div>
  );
}

function WheelsBrakes({ parts }: { parts: TechLogPart[] }) {
  const tyres = parts.filter(isTyre);
  const brakes = parts.filter(isBrake);
  if (tyres.length === 0) return null;
  const byLabel = (a: TechLogPart, b: TechLogPart) => tyreLabel(a).localeCompare(tyreLabel(b), undefined, { numeric: true });
  const nose = tyres.filter(isNoseTyre).sort(byLabel);
  const mains = tyres.filter((p) => !isNoseTyre(p)).sort(byLabel);
  const half = Math.ceil(mains.length / 2);
  const brakeFor = (p: TechLogPart) => brakes.find((b) => (b.position ?? "") === (p.position ?? "__"));
  return (
    <section className="self-start rounded-xl border border-sf-border bg-sf-bg/40 p-4">
      <header className="mb-3 flex items-center justify-between gap-2">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("tlv.wheels")}</h3>
        <span className="text-[10px] uppercase tracking-wide text-sf-muted">{t("tlv.wheels_hint")}</span>
      </header>
      <div className="flex flex-col items-center gap-3">
        <div className="flex gap-4">{nose.map((p) => <Tyre key={p.id} p={p} />)}</div>
        <div className="flex gap-10">
          {[mains.slice(0, half), mains.slice(half)].map((side, i) => (
            <div key={i} className="flex gap-3">
              {side.map((p) => (
                <Tyre key={p.id} p={p} brake={brakeFor(p)} />
              ))}
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------------------
// Vista completa
// ---------------------------------------------------------------------------

export function TechLogStatus({
  data,
  model,
  renderActions,
}: {
  data: TechLogAircraft;
  /** Modelo o título del avión (para elegir la silueta). */
  model?: string | null;
  /** Botones de mantenimiento de una pieza (los mismos de la lista del Hangar). */
  renderActions: (p: TechLogPart) => ReactNode;
}) {
  const parts = data.allParts ?? data.parts;
  const kind = silhouetteFor(data.variant, model);
  // La disposición depende del ancho del panel (el Hangar es más estrecho
  // que la ventana): ancho → como TechLog (avión | motores y ruedas);
  // medio → avión | ruedas y motores debajo; estrecho → todo apilado.
  const boxRef = useRef<HTMLDivElement | null>(null);
  const [width, setWidth] = useState(1200);
  useEffect(() => {
    const el = boxRef.current;
    if (!el) return;
    const ro = new ResizeObserver(([e]) => setWidth(e.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const layout = width >= 1150 ? "wide" : width >= 640 ? "medium" : "narrow";
  // Cada motor necesita ~380 px para que se lean sus secciones.
  const engineArea = layout === "wide" ? (width * 7) / 12 : width;
  const engineCols = engineArea >= 780 ? 2 : 1;
  const [engineSel, setEngineSel] = useState<number | null>(null);
  const [system, setSystem] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const systems = useMemo(() => groupSystems(parts), [parts]);
  const engineCount = Math.max(
    enginesOf(shapeOf(kind)).length === 4 ? 4 : 2,
    data.egtMarginC.length,
    ...parts.map((p) => engineOf(p) ?? 0),
  );
  const engineNums = Array.from({ length: engineCount }, (_, i) => i + 1).filter(
    (n) => n <= 2 || parts.some((p) => engineOf(p) === n) || data.egtMarginC[n - 1] != null,
  );
  const stats = data.stats;
  const selected = systems.find((g) => g.id === system) ?? null;
  const q = query.trim().toLowerCase();
  const found = q ? parts.filter((p) => `${p.name} ${p.system} ${p.position ?? ""}`.toLowerCase().includes(q)) : [];

  const openSystem = (id: string) => {
    setSystem(id);
    requestAnimationFrame(() => document.getElementById("tlv-system-detail")?.scrollIntoView({ block: "nearest", behavior: "smooth" }));
  };
  const counters: { label: string; value: string }[] = [
    { label: t("tlv.flight_hours"), value: num(data.flightHours, 1) },
    { label: t("tlv.block_hours"), value: stats ? num(stats.blockHours, 1) : "—" },
    { label: t("tlv.cycles"), value: num(data.cycles) },
    { label: t("tlv.landings"), value: stats ? num(stats.landings) : "—" },
    { label: t("tlv.apu_hours"), value: stats ? num(stats.apuHours, 1) : "—" },
    { label: t("tlv.apu_starts"), value: stats ? num(stats.apuStarts) : "—" },
  ];

  const aircraftPanel = (
        <section className="flex flex-col rounded-xl border border-sf-border bg-sf-bg/40 p-4">
          <h3 className="text-[11px] font-semibold uppercase tracking-wide text-sf-muted">
            {[data.variant, data.engine, data.registration].filter(Boolean).join(" · ")}
          </h3>
          <div className="flex flex-1 items-center py-2">
            <AircraftDiagram
              kind={kind}
              data={data}
              parts={parts}
              engineSel={engineSel}
              onEngine={(n) => {
                setEngineSel(n === engineSel ? null : n);
                document.getElementById(`tlv-engine-${n}`)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
              }}
              onSystem={openSystem}
            />
          </div>
          <div className="grid grid-cols-3 gap-1.5">
            {counters.map((c) => (
              <div key={c.label} className="rounded-lg border border-sf-border/70 bg-sf-raised/40 px-2 py-1.5">
                <div className="text-[9px] uppercase tracking-wide text-sf-muted">{c.label}</div>
                <div className="font-mono text-sm font-semibold text-sf-text">{c.value}</div>
              </div>
            ))}
          </div>
        </section>
  );
  const enginesPanel = (
          <section className="rounded-xl border border-sf-border bg-sf-bg/40 p-4">
            <h3 className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-sf-muted">{t("tlv.engines")}</h3>
            <div className={cn("grid gap-3", engineCols === 2 ? "grid-cols-2" : "grid-cols-1")}>
              {engineNums.map((n) => (
                <div key={n} id={`tlv-engine-${n}`}>
                  <EngineCard
                    n={n}
                    parts={parts.filter((p) => engineOf(p) === n)}
                    egt={data.egtMarginC[n - 1]}
                    hours={stats?.engHours[n - 1]}
                    cycles={stats?.engCycles[n - 1]}
                    starts={stats?.engStarts[n - 1]}
                    selected={engineSel === n}
                    onSelect={() => {
                      setEngineSel(n);
                      openSystem(`eng${n}`);
                    }}
                  />
                </div>
              ))}
            </div>
          </section>
  );
  const wheelsPanel = <WheelsBrakes parts={parts} />;

  return (
    <div ref={boxRef} className="space-y-4">
      {layout === "wide" ? (
        <div className="grid grid-cols-[minmax(0,5fr)_minmax(0,7fr)] gap-4">
          {aircraftPanel}
          <div className="space-y-4">
            {enginesPanel}
            {wheelsPanel}
          </div>
        </div>
      ) : layout === "medium" ? (
        <>
          <div className={cn("grid gap-4", parts.some(isTyre) ? "grid-cols-2" : "grid-cols-1")}>
            {aircraftPanel}
            {wheelsPanel}
          </div>
          {enginesPanel}
        </>
      ) : (
        <>
          {aircraftPanel}
          {enginesPanel}
          {wheelsPanel}
        </>
      )}

      {/* Sistemas */}
      <section className="rounded-xl border border-sf-border bg-sf-bg/40 p-4">
        <header className="mb-3 flex flex-wrap items-center gap-2">
          <h3 className="text-[11px] font-semibold uppercase tracking-wide text-sf-muted">
            {t("tlv.systems", { n: String(parts.length) })}
          </h3>
          <span className="ml-auto text-[10px] uppercase tracking-wide text-sf-muted">{t("tlv.systems_hint")}</span>
          <label className="relative">
            <Search className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-sf-muted" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("tlv.find_part")}
              className="w-48 rounded-md border border-sf-border bg-sf-surface py-1 pl-6 pr-2 text-xs text-sf-text placeholder:text-sf-muted focus:border-brand-500/60 focus:outline-none"
            />
          </label>
        </header>
        <div className="grid grid-cols-[repeat(auto-fill,minmax(170px,1fr))] gap-2">
          {systems.map((g) => {
            const failed = g.parts.filter((p) => partTone(p) === "failed").length;
            const bad = g.parts.filter((p) => partTone(p) !== "ok").length;
            return (
              <button
                key={g.id}
                onClick={() => (system === g.id ? setSystem(null) : openSystem(g.id))}
                className={cn(
                  "rounded-lg border border-l-[3px] bg-sf-raised/30 p-2 text-left transition-colors hover:bg-sf-raised/60",
                  system === g.id ? "border-slate-300/70" : "border-sf-border",
                )}
                style={{ borderLeftColor: TONE_FILL[g.tone] }}
              >
                <div className="flex items-baseline justify-between gap-1">
                  <span className="font-mono text-[11px] font-bold text-sf-text">{g.code}</span>
                  <span className="text-[9px] text-sf-muted">{g.ata}</span>
                </div>
                <div className="truncate text-[10px] text-sf-muted">{g.label}</div>
                <div className="mt-1.5 flex flex-wrap gap-[3px]">
                  {g.parts.map((p) => (
                    <span
                      key={p.id}
                      title={`${p.name} · ${pct(p.remainingPct)}`}
                      className="h-2 w-2 rounded-[2px]"
                      style={{ backgroundColor: TONE_FILL[partTone(p)] }}
                    />
                  ))}
                </div>
                <div className={cn("mt-1.5 text-[10px]", failed ? "text-rose-300" : bad ? "text-amber-300" : "text-sf-muted")}>
                  {failed
                    ? t(failed === 1 ? "tlv.sys_failed_one" : "tlv.sys_failed", { n: String(failed) })
                    : bad
                      ? t("tlv.sys_worn", { n: String(bad), total: String(g.parts.length) })
                      : t("tlv.sys_ok", { n: String(g.parts.length) })}
                </div>
              </button>
            );
          })}
        </div>
        {(selected || q) && (
          <div id="tlv-system-detail" className="mt-3 rounded-lg border border-sf-border bg-sf-surface/60">
            <div className="flex items-center gap-2 border-b border-sf-border px-3 py-2">
              <span className="text-xs font-semibold text-sf-text">
                {q ? t("tlv.search_results", { n: String(found.length) }) : `${selected!.code} · ${selected!.label} (${selected!.parts.length})`}
              </span>
              <button
                onClick={() => {
                  setSystem(null);
                  setQuery("");
                }}
                className="ml-auto rounded-md p-1 text-sf-muted hover:bg-sf-raised hover:text-sf-text"
                title={t("common.close")}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
            <ul className="max-h-80 divide-y divide-sf-border/50 overflow-y-auto">
              {(q ? found : selected!.parts).map((p) => (
                <li key={p.id} className="flex flex-wrap items-center gap-2 px-3 py-1.5">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-xs text-sf-text">{p.name}</div>
                    <div className="text-[10px] text-sf-muted">
                      ATA {p.ata} · {p.system}
                      {p.status !== "Ok" && (
                        <span className={cn("ml-1 font-semibold", TONE_TEXT[partTone(p)])}>
                          · {t(p.status === "Failed" ? "techlog.parts.failed" : "techlog.parts.degraded")}
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="flex w-44 items-center gap-1.5">
                    <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-sf-raised">
                      <div
                        className="h-full rounded-full"
                        style={{ width: `${Math.max(2, p.remainingPct)}%`, backgroundColor: TONE_FILL[partTone(p)] }}
                      />
                    </div>
                    <span className="w-24 whitespace-nowrap text-right text-[10px] tabular-nums text-sf-muted">
                      {t("tlv.left", { p: pct(p.remainingPct) })}
                    </span>
                  </div>
                  <div className="flex shrink-0 gap-1">{renderActions(p)}</div>
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>
    </div>
  );
}
