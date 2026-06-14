import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  useEdgesState,
  useNodesState,
  type Connection,
  type Edge,
  type Node,
  type NodeProps,
  type ReactFlowInstance,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import dagre from "@dagrejs/dagre";
import {
  Cog,
  GitBranch,
  HelpCircle,
  LayoutGrid,
  Link2,
  MapPin,
  Music,
  Palette,
  Plane,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import type {
  AddonLink,
  AddonNodePosition,
  CommunityPackage,
} from "../lib/types";
import type { DerivedType } from "../lib/packageType";
import { looksLikePlaceholderTitle } from "../lib/packageType";
import { useThumbnail } from "../lib/thumbnails";
import { ToggleSwitch, usePackageToggle } from "./AddonToggle";
import { AddonFallbackArt } from "./AddonArt";
import { FindSearch } from "./FindSearch";
import { RegionBadge } from "./RegionBadge";
import { PerfBadge } from "./PerfBadge";
import { usePerfStore } from "../stores/usePerfStore";
import { airportRegion, continentLabel, type Continent } from "../lib/oaciRegion";
import { expandVendorQuery } from "../lib/vendorSynonyms";
import { api } from "../lib/tauri";
import { useToastStore } from "../stores/useToastStore";
import { t } from "../lib/i18n";

/**
 * (v4.25.0) **Link Map** — grafo dirigido de dependencias de addons.
 *
 * · Cada nodo es una card del addon (thumbnail + badge + toggle).
 * · Las aristas punteadas ámbar van del paquete base (aircraft) a sus
 *   dependientes (liveries, soundpacks, tweaks). Las 'auto' se
 *   siembran del manifest en cada scan; el usuario crea 'manual'
 *   arrastrando del punto verde (salida) al punto azul (entrada).
 * · Encender un nodo raíz enciende EN CASCADA todo lo enlazado
 *   (transitivo, cycle-safe — BFS en el backend). El estado fluye por
 *   el store compartido, así que los switches de los nodos alcanzados
 *   se animan solos.
 * · Membresía del lienzo: addons con al menos un link + los añadidos
 *   a mano con "Add Item" (persisten por su posición guardada).
 * · Posiciones: se guardan en DB al soltar un drag; los nodos sin
 *   posición se acomodan con dagre (izquierda → derecha).
 */

interface AddonItem {
  p: CommunityPackage;
  t: DerivedType;
}

const NODE_W = 200;
const NODE_H = 140;

// ---------------------------------------------------------------------------
// Nodo custom — card de addon con handles y toggle
// ---------------------------------------------------------------------------

type AddonNodeType = Node<{ pkg: CommunityPackage; derived: DerivedType }, "addon">;

function AddonNode({ data }: NodeProps<AddonNodeType>) {
  const { pkg, derived } = data;
  const skipThumb = looksLikePlaceholderTitle(pkg.title);
  const thumb = useThumbnail(pkg.folderName, skipThumb);
  const { enabled, busy, toggle } = usePackageToggle(pkg);

  return (
    <div
      className={`w-[200px] overflow-hidden rounded-lg border bg-slate-950 shadow-lg transition-colors ${
        enabled ? "border-slate-700" : "border-slate-800 opacity-70"
      }`}
    >
      {/* Handle de ENTRADA (izquierda, azul): este addon depende de… */}
      <Handle
        type="target"
        position={Position.Left}
        className="!h-2.5 !w-2.5 !border-2 !border-slate-950 !bg-sky-400"
      />
      {/* Handle de SALIDA (derecha, verde): …addons que dependen de este. */}
      <Handle
        type="source"
        position={Position.Right}
        className="!h-2.5 !w-2.5 !border-2 !border-slate-950 !bg-emerald-400"
      />

      <div className="relative h-[72px] w-full overflow-hidden bg-gradient-to-br from-slate-800 to-slate-950">
        {thumb ? (
          <img
            src={thumb}
            alt=""
            loading="lazy"
            decoding="async"
            className={`h-full w-full object-cover ${enabled ? "" : "opacity-40 grayscale"}`}
            onError={(e) => {
              (e.currentTarget as HTMLImageElement).style.display = "none";
            }}
          />
        ) : (
          // (v4.26.0) Sin thumbnail → mismo arte tipado que el grid.
          <div className={`h-full w-full ${enabled ? "" : "opacity-40 grayscale"}`}>
            <AddonFallbackArt derived={derived} title={pkg.title} />
          </div>
        )}
        <span className={nodeBadgeClass(derived)}>{nodeBadgeLabel(derived)}</span>
      </div>
      <div className="space-y-1 p-2">
        <p
          className={`line-clamp-1 text-[11px] font-semibold leading-tight ${
            enabled ? "text-slate-100" : "text-slate-500"
          }`}
          title={pkg.title}
        >
          {pkg.title}
        </p>
        <p className="line-clamp-1 text-[9px] text-slate-500">
          {pkg.creator ?? "—"}
          {pkg.packageVersion && ` · v${pkg.packageVersion}`}
        </p>
        <div className="flex items-center justify-between pt-0.5">
          <ToggleSwitch on={enabled} busy={busy} onToggle={toggle} small />
          <span className="text-[8px] uppercase tracking-wide text-slate-600">
            {pkg.folderName.slice(0, 22)}
          </span>
        </div>
      </div>
    </div>
  );
}

// (v4.33.0 · v5.0.0) Nodo de AEROPUERTO — visualmente distinto del addon
// (borde verde, pin, ICAO). Standalone, sin aristas. Se agrupa por
// continente en clusters a la derecha del lienzo. Lleva micro-badge de
// región + toggle nativo para encender/apagar el escenario sin salir.
type AirportNodeType = Node<{ pkg: CommunityPackage }, "airport">;

function AirportNode({ data }: NodeProps<AirportNodeType>) {
  const { pkg } = data;
  const thumb = useThumbnail(pkg.folderName, false);
  const { enabled, busy, toggle } = usePackageToggle(pkg);
  const isOptimizable = usePerfStore((s) => s.optimizable.has(pkg.folderName));
  return (
    <div
      className={`w-[180px] overflow-hidden rounded-lg border-2 shadow-lg transition-colors ${
        enabled
          ? "border-emerald-600/60 bg-emerald-950/40"
          : "border-slate-700 bg-slate-950/70 opacity-70"
      }`}
    >
      <div className="relative h-[56px] w-full overflow-hidden bg-gradient-to-br from-emerald-900/40 to-slate-950">
        {thumb ? (
          <img
            src={thumb}
            alt=""
            className={`h-full w-full object-cover ${enabled ? "" : "opacity-40 grayscale"}`}
          />
        ) : (
          <div className="flex h-full w-full items-center justify-center text-emerald-500/40">
            <MapPin className="h-7 w-7" />
          </div>
        )}
        <span className="absolute left-1.5 top-1.5 rounded bg-emerald-500/90 px-1 py-0.5 font-mono text-[9px] font-bold text-emerald-950">
          {pkg.icao}
        </span>
        <span className="absolute right-1.5 top-1.5 flex flex-col items-end gap-1">
          <RegionBadge icao={pkg.icao} showLabel={false} />
          {isOptimizable && <PerfBadge showLabel={false} />}
        </span>
      </div>
      <div className="flex items-center gap-1.5 p-2">
        <p
          className={`line-clamp-1 min-w-0 flex-1 text-[10px] font-semibold ${
            enabled ? "text-emerald-100" : "text-slate-500"
          }`}
          title={pkg.airportName ?? pkg.title}
        >
          {pkg.airportName ?? pkg.title}
        </p>
        <ToggleSwitch on={enabled} busy={busy} onToggle={toggle} small />
      </div>
    </div>
  );
}

// (v5.0.0) Cabecera de cluster: rótulo del continente sobre cada grupo
// de aeropuertos. Nodo no interactivo (solo etiqueta visual).
type RegionHeaderNodeType = Node<{ label: string; count: number }, "regionHeader">;

function RegionHeaderNode({ data }: NodeProps<RegionHeaderNodeType>) {
  return (
    <div className="pointer-events-none flex items-center gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-1.5 shadow-sm">
      <MapPin className="h-3.5 w-3.5 text-emerald-300" />
      <span className="text-[13px] font-bold uppercase tracking-wide text-emerald-200">
        {data.label}
      </span>
      <span className="rounded-full bg-emerald-500/20 px-2 py-0.5 text-[10px] font-bold tabular-nums text-emerald-200">
        {data.count}
      </span>
    </div>
  );
}

const NODE_TYPES = {
  addon: AddonNode,
  airport: AirportNode,
  regionHeader: RegionHeaderNode,
};
type FlowNode = AddonNodeType | AirportNodeType | RegionHeaderNodeType;

// ---------------------------------------------------------------------------
// Vista principal
// ---------------------------------------------------------------------------

export function LinkMapView({
  addons,
  airports = [],
}: {
  addons: AddonItem[];
  airports?: CommunityPackage[];
}) {
  const pushToast = useToastStore((s) => s.push);
  const [links, setLinks] = useState<AddonLink[]>([]);
  const [findActive, setFindActive] = useState(1);
  const [positions, setPositions] = useState<Map<string, AddonNodePosition>>(
    new Map(),
  );
  const [loaded, setLoaded] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [addQuery, setAddQuery] = useState("");
  const [findQuery, setFindQuery] = useState("");
  const flowRef = useRef<ReactFlowInstance<FlowNode, Edge> | null>(null);

  const byFolder = useMemo(() => {
    const m = new Map<string, AddonItem>();
    for (const item of addons) m.set(item.p.folderName, item);
    return m;
  }, [addons]);

  const reloadGraph = useCallback(async () => {
    try {
      const [ls, ps] = await Promise.all([
        api.listAddonLinks(),
        api.listAddonNodePositions(),
      ]);
      setLinks(ls);
      setPositions(new Map(ps.map((p) => [p.folderName, p])));
      setLoaded(true);
    } catch (e) {
      pushToast({
        kind: "error",
        title: t("linkmap.load_error"),
        message: String(e),
      });
    }
  }, [pushToast]);

  useEffect(() => {
    void reloadGraph();
  }, [reloadGraph]);

  // (v5.0.0) Escanea qué aeropuertos son optimizables (badge tuerca en
  // sus nodos). Idempotente y cacheado por folderName.
  const ensurePerf = usePerfStore((s) => s.ensure);
  useEffect(() => {
    if (airports.length === 0) return;
    void ensurePerf(
      airports.map((ap) => ({
        folderName: ap.folderName,
        installPath: ap.installPath,
      })),
    );
  }, [airports, ensurePerf]);

  // (v4.28.0) Membresía del lienzo: TODOS los addons del inventario
  // — el usuario pidió que el Link Map muestre todo, no solo lo
  // linkeado. Antes solo aparecían los que estaban en algún link o
  // tenían posición guardada.
  const members = useMemo(
    () => addons.map((a) => a.p.folderName),
    [addons],
  );

  const visibleLinks = useMemo(
    () =>
      links.filter(
        (l) => byFolder.has(l.sourceFolder) && byFolder.has(l.targetFolder),
      ),
    [links, byFolder],
  );

  const [nodes, setNodes, onNodesChange] = useNodesState<FlowNode>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);

  // (v5.0.0) Zona de aeropuertos: clusters compactos por CONTINENTE
  // (grilla de pocas columnas + cabecera con rótulo) apilados en una
  // franja a la DERECHA de los addons, físicamente aislada de los
  // flujos de aviones/liveries. No se persisten (siempre auto-ordenados).
  const airportLayout = useMemo(() => layoutAirportZone(airports), [airports]);

  // Reconstruye nodos/aristas cuando cambian membresía, links o el
  // inventario (un toggle refresca `enabled` dentro de data.pkg).
  // Posición: DB → posición actual en el lienzo → dagre.
  useEffect(() => {
    if (!loaded) return;
    setNodes((current) => {
      const currentPos = new Map(current.map((n) => [n.id, n.position]));
      const dagrePos = layoutWithDagre(
        members.filter((m) => !positions.has(m) && !currentPos.has(m)),
        members,
        visibleLinks,
        byFolder,
      );
      const addonNodes: FlowNode[] = members.map((folder) => {
        const item = byFolder.get(folder)!;
        const pos =
          positions.get(folder) ??
          (currentPos.get(folder)
            ? { folderName: folder, ...currentPos.get(folder)! }
            : null);
        const fallback = dagrePos.get(folder) ?? { x: 0, y: 0 };
        return {
          id: folder,
          type: "addon" as const,
          position: pos ? { x: pos.x, y: pos.y } : fallback,
          // Supr solo borra ARISTAS: la membresía se recalcula de
          // links+posiciones y un nodo "borrado" reaparecería en el
          // próximo rebuild — mejor no permitirlo.
          deletable: false,
          data: { pkg: item.p, derived: item.t },
        };
      });
      // (v5.0.0) Nodos de aeropuerto — clusters por continente a la
      // derecha, con sus cabeceras de región.
      const airportNodes: FlowNode[] = airports.map((ap) => {
        const p = airportLayout.positions.get(ap.folderName) ?? { x: 0, y: 0 };
        return {
          id: `airport:${ap.folderName}`,
          type: "airport" as const,
          position: { x: p.x, y: p.y },
          deletable: false,
          draggable: false,
          connectable: false,
          data: { pkg: ap },
        };
      });
      const headerNodes: FlowNode[] = airportLayout.headers.map((h) => ({
        id: h.id,
        type: "regionHeader" as const,
        position: { x: h.x, y: h.y },
        deletable: false,
        draggable: false,
        selectable: false,
        connectable: false,
        data: { label: h.label, count: h.count },
      }));
      return [...addonNodes, ...airportNodes, ...headerNodes];
    });
    setEdges(
      visibleLinks.map((l) => ({
        id: `${l.sourceFolder}->${l.targetFolder}`,
        source: l.sourceFolder,
        target: l.targetFolder,
        // (v5.0.0) smoothstep + trazo más fino y tenue: con cientos de
        // links las curvas rectas se cruzaban como ruido sobre las
        // tarjetas. Las ortogonales suaves leen mejor y van por debajo.
        type: "smoothstep",
        animated: true,
        style: { stroke: "#f59e0b", strokeWidth: 1.25, strokeOpacity: 0.55 },
        markerEnd: { type: MarkerType.ArrowClosed, color: "#f59e0b", width: 14, height: 14 },
      })),
    );
  }, [loaded, members, visibleLinks, byFolder, positions, airports, airportLayout, setNodes, setEdges]);

  // Crear arista arrastrando verde → azul.
  const onConnect = useCallback(
    async (conn: Connection) => {
      if (!conn.source || !conn.target || conn.source === conn.target) return;
      try {
        await api.addAddonLink(conn.source, conn.target);
        await reloadGraph();
      } catch (e) {
        pushToast({
          kind: "error",
          title: t("linkmap.link_error"),
          message: String(e),
        });
      }
    },
    [reloadGraph, pushToast],
  );

  // Borrar aristas seleccionadas (tecla Delete / Backspace).
  const onEdgesDelete = useCallback(
    async (deleted: Edge[]) => {
      try {
        for (const e of deleted) {
          await api.removeAddonLink(e.source, e.target);
        }
        await reloadGraph();
      } catch (e) {
        pushToast({
          kind: "error",
          title: t("linkmap.link_error"),
          message: String(e),
        });
      }
    },
    [reloadGraph, pushToast],
  );

  // Persistir posición al soltar el drag.
  const onNodeDragStop = useCallback(
    (_: unknown, node: FlowNode) => {
      // Los aeropuertos no se arrastran (draggable:false) ni se
      // persisten — solo addons.
      if (node.type !== "addon") return;
      const pos: AddonNodePosition = {
        folderName: node.id,
        x: node.position.x,
        y: node.position.y,
      };
      setPositions((prev) => new Map(prev).set(node.id, pos));
      void api.saveAddonNodePositions([pos]).catch(() => {});
    },
    [],
  );

  // "Add Item": añade un addon suelto al lienzo persistiendo una
  // posición cerca del centro actual del viewport.
  const addItem = async (folder: string) => {
    const inst = flowRef.current;
    let x = 0;
    let y = 0;
    if (inst) {
      const vp = inst.getViewport();
      // Centro del viewport en coordenadas del lienzo + jitter para
      // que múltiples adds no caigan apilados exactos.
      x = (-vp.x + 380) / vp.zoom + Math.random() * 60 - 30;
      y = (-vp.y + 200) / vp.zoom + Math.random() * 60 - 30;
    }
    const pos: AddonNodePosition = { folderName: folder, x, y };
    setPositions((prev) => new Map(prev).set(folder, pos));
    setAddOpen(false);
    setAddQuery("");
    await api.saveAddonNodePositions([pos]).catch(() => {});
  };

  // (v4.33.0) Auto-organizar: recalcula la posición de TODOS los nodos
  // de addon con el layout automático (clusters por marca) ignorando lo
  // que hubiera guardado, lo persiste y encuadra la cámara. El usuario
  // pidió no tener que arrastrar uno por uno.
  const autoArrange = async () => {
    const fresh = layoutWithDagre(members, members, visibleLinks, byFolder);
    const newPositions = new Map<string, AddonNodePosition>();
    for (const folder of members) {
      const p = fresh.get(folder);
      if (p) newPositions.set(folder, { folderName: folder, x: p.x, y: p.y });
    }
    setPositions(newPositions);
    setNodes((cur) =>
      cur.map((n) => {
        const p = fresh.get(n.id);
        return p ? { ...n, position: { x: p.x, y: p.y } } : n;
      }),
    );
    await api
      .saveAddonNodePositions([...newPositions.values()])
      .catch(() => {});
    setTimeout(
      () => flowRef.current?.fitView({ padding: 0.2, duration: 600 }),
      60,
    );
    pushToast({ kind: "success", title: t("linkmap.arranged"), ttlMs: 2500 });
  };

  // (v4.33.0) Find-in-page del lienzo: lista de nodos que matchean +
  // navegación prev/next que centra la cámara en cada uno.
  const findMatches = useMemo(() => {
    const needle = findQuery.trim().toLowerCase();
    if (!needle) return [] as FlowNode[];
    const expanded = expandVendorQuery(needle);
    return nodes.filter((n) => {
      if (n.type === "regionHeader") return false;
      const hay = `${n.data.pkg.title} ${n.id}`.toLowerCase();
      return expanded.some((term) => hay.includes(term));
    });
  }, [findQuery, nodes]);

  const centerOnNode = useCallback((n: FlowNode) => {
    if (!flowRef.current) return;
    flowRef.current.setCenter(
      n.position.x + NODE_W / 2,
      n.position.y + NODE_H / 2,
      { zoom: 1.1, duration: 500 },
    );
  }, []);

  const stepFind = (delta: number) => {
    const total = findMatches.length;
    if (total === 0) return;
    const next = ((findActive - 1 + delta + total) % total) + 1;
    setFindActive(next);
    centerOnNode(findMatches[next - 1]);
  };

  // Al cambiar la query, resetea al primer match y centra en él.
  const onFindChange = (q: string) => {
    setFindQuery(q);
    setFindActive(1);
    const needle = q.trim().toLowerCase();
    if (!needle) return;
    const expanded = expandVendorQuery(needle);
    const hit = nodes.find((n) => {
      if (n.type === "regionHeader") return false;
      const hay = `${n.data.pkg.title} ${n.id}`.toLowerCase();
      return expanded.some((term) => hay.includes(term));
    });
    if (hit) centerOnNode(hit);
  };

  const addCandidates = useMemo(() => {
    const onCanvas = new Set(members);
    const q = addQuery.trim().toLowerCase();
    return addons
      .filter(({ p }) => !onCanvas.has(p.folderName))
      .filter(
        ({ p }) =>
          !q ||
          p.title.toLowerCase().includes(q) ||
          p.folderName.toLowerCase().includes(q),
      )
      .slice(0, 30);
  }, [addons, members, addQuery]);

  return (
    // (v4.27.0) Pantalla casi completa — antes el lienzo quedaba con
    // ~480px y tocaba dar zoom + arrastrar para ver todo. Ahora el
    // contenedor llena el viewport descontando el header + nav
    // sticky + métricas + chips de vista (~10rem).
    <div className="relative h-[calc(100vh-11rem)] min-h-[560px] overflow-hidden rounded-2xl border border-slate-800 bg-slate-950/60">
      {/* Barra superior del lienzo — contadores + acciones. */}
      <div className="absolute left-3 top-3 z-10 flex items-center gap-2">
        <span className="inline-flex items-center gap-1.5 rounded-md bg-slate-950/80 px-2.5 py-1.5 text-[11px] text-slate-300 ring-1 ring-slate-800 backdrop-blur">
          <GitBranch className="h-3 w-3 text-amber-300" />
          <b className="tabular-nums text-slate-100">{members.length}</b>
          {t("linkmap.addons_label")}
          <span className="text-slate-600">·</span>
          <b className="tabular-nums text-slate-100">{visibleLinks.length}</b>
          {t("linkmap.links_label")}
        </span>
      </div>

      {/* (v4.33.0) Find-in-page del lienzo: contador + flechas que
          centran cada nodo coincidente. */}
      <div className="absolute left-1/2 top-3 z-10 -translate-x-1/2">
        <FindSearch
          value={findQuery}
          onChange={onFindChange}
          total={findMatches.length}
          active={findMatches.length === 0 ? 0 : findActive}
          onPrev={() => stepFind(-1)}
          onNext={() => stepFind(1)}
          onClose={() => onFindChange("")}
          placeholder={t("linkmap.find_placeholder")}
        />
      </div>

      <div className="absolute right-3 top-3 z-10 flex items-center gap-2">
        {/* (v4.33.0) Auto-organizar: reposiciona TODOS los nodos con el
            layout automático. El usuario no mueve uno por uno. */}
        <button
          onClick={() => void autoArrange()}
          title={t("linkmap.auto_arrange")}
          className="inline-flex items-center gap-1.5 rounded-lg bg-slate-700 px-2.5 py-1.5 text-[11px] font-semibold text-slate-100 shadow-md hover:bg-slate-600"
        >
          <LayoutGrid className="h-3 w-3" />
          {t("linkmap.auto_arrange")}
        </button>
        <button
          onClick={() => setManageOpen(true)}
          className="inline-flex items-center gap-1.5 rounded-lg bg-amber-500 px-2.5 py-1.5 text-[11px] font-semibold text-amber-950 shadow-md shadow-amber-500/20 hover:bg-amber-400"
        >
          <Link2 className="h-3 w-3" />
          {t("linkmap.manage_links")}
        </button>
        <button
          onClick={() => setAddOpen((v) => !v)}
          className="inline-flex items-center gap-1.5 rounded-lg bg-emerald-500 px-2.5 py-1.5 text-[11px] font-semibold text-emerald-950 shadow-md shadow-emerald-500/20 hover:bg-emerald-400"
        >
          <Plus className="h-3 w-3" />
          {t("linkmap.add_item")}
        </button>
      </div>

      {/* Popover "Add Item" — addons fuera del lienzo, con buscador. */}
      {addOpen && (
        <div className="absolute right-3 top-12 z-20 w-72 overflow-hidden rounded-xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800">
          <div className="flex items-center gap-2 border-b border-slate-800 px-3 py-2">
            <Search className="h-3 w-3 shrink-0 text-slate-500" />
            <input
              autoFocus
              value={addQuery}
              onChange={(e) => setAddQuery(e.target.value)}
              placeholder={t("linkmap.add_placeholder")}
              className="w-full bg-transparent text-[11px] text-slate-200 placeholder:text-slate-600 focus:outline-none"
            />
            <button
              onClick={() => setAddOpen(false)}
              className="rounded p-0.5 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
            >
              <X className="h-3 w-3" />
            </button>
          </div>
          <ul className="max-h-72 overflow-y-auto py-1">
            {addCandidates.length === 0 ? (
              <li className="px-3 py-4 text-center text-[11px] text-slate-600">
                {t("linkmap.add_empty")}
              </li>
            ) : (
              addCandidates.map(({ p, t: ty }) => (
                <li key={p.folderName}>
                  <button
                    onClick={() => void addItem(p.folderName)}
                    className="flex w-full items-center gap-2 px-3 py-1.5 text-left hover:bg-slate-800/60"
                  >
                    <span className="shrink-0 text-slate-500">
                      {nodeTypeIcon(ty, "h-3 w-3")}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[11px] text-slate-200">
                      {p.title}
                    </span>
                  </button>
                </li>
              ))
            )}
          </ul>
        </div>
      )}

      {/* Lienzo React Flow. */}
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        onEdgesDelete={onEdgesDelete}
        onNodeDragStop={onNodeDragStop}
        onInit={(inst) => {
          flowRef.current = inst;
        }}
        fitView
        fitViewOptions={{ padding: 0.25, maxZoom: 1 }}
        minZoom={0.2}
        maxZoom={2}
        proOptions={{ hideAttribution: true }}
        deleteKeyCode={["Delete", "Backspace"]}
        nodesConnectable
        colorMode="dark"
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={22}
          size={1.5}
          color="#1e293b"
        />
        <Controls position="bottom-left" showInteractive={false} />
      </ReactFlow>

      {/* Estado vacío — guía rápida. */}
      {loaded && members.length === 0 && (
        <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center">
          <div className="pointer-events-auto max-w-sm rounded-xl border border-slate-800 bg-slate-950/90 p-5 text-center backdrop-blur">
            <GitBranch className="mx-auto mb-2 h-7 w-7 text-amber-400" />
            <p className="text-sm font-semibold text-slate-100">
              {t("linkmap.empty.title")}
            </p>
            <p className="mt-1.5 text-xs text-slate-400">
              {t("linkmap.empty.body")}
            </p>
            <button
              onClick={() => setAddOpen(true)}
              className="mt-3 inline-flex items-center gap-1.5 rounded-lg bg-emerald-500 px-3 py-1.5 text-xs font-semibold text-emerald-950 hover:bg-emerald-400"
            >
              <Plus className="h-3.5 w-3.5" />
              {t("linkmap.add_item")}
            </button>
          </div>
        </div>
      )}

      {/* Hint de conexión, abajo al centro (estilo barra de atajos). */}
      <div className="pointer-events-none absolute bottom-3 left-1/2 z-10 -translate-x-1/2 rounded-md bg-slate-950/80 px-3 py-1 text-[10px] text-slate-500 ring-1 ring-slate-800 backdrop-blur">
        {t("linkmap.hint")}
      </div>

      {manageOpen && (
        <ManageLinksModal
          links={links}
          addons={addons}
          byFolder={byFolder}
          onChanged={reloadGraph}
          onClose={() => setManageOpen(false)}
        />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Modal "Manage Links"
// ---------------------------------------------------------------------------

function ManageLinksModal({
  links,
  addons,
  byFolder,
  onChanged,
  onClose,
}: {
  links: AddonLink[];
  addons: AddonItem[];
  byFolder: Map<string, AddonItem>;
  onChanged: () => Promise<void>;
  onClose: () => void;
}) {
  const pushToast = useToastStore((s) => s.push);
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);

  const sorted = useMemo(
    () =>
      [...addons].sort((a, b) => a.p.title.localeCompare(b.p.title)),
    [addons],
  );

  const titleOf = (folder: string) =>
    byFolder.get(folder)?.p.title ?? folder;

  const addLink = async () => {
    if (!source || !target || source === target) return;
    setBusy(true);
    try {
      await api.addAddonLink(source, target);
      await onChanged();
      setSource("");
      setTarget("");
    } catch (e) {
      pushToast({
        kind: "error",
        title: t("linkmap.link_error"),
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  const removeLink = async (l: AddonLink) => {
    setBusy(true);
    try {
      await api.removeAddonLink(l.sourceFolder, l.targetFolder);
      await onChanged();
    } catch (e) {
      pushToast({
        kind: "error",
        title: t("linkmap.link_error"),
        message: String(e),
      });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/70 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[min(640px,calc(100vh-4rem))] w-[min(620px,calc(100vw-2rem))] flex-col overflow-hidden rounded-2xl border border-slate-800 bg-slate-950 shadow-2xl ring-1 ring-slate-800"
      >
        <header className="flex items-center justify-between border-b border-slate-800 px-5 py-3">
          <h3 className="inline-flex items-center gap-2 text-sm font-semibold text-amber-200">
            <Link2 className="h-4 w-4" />
            {t("linkmap.modal.title")}
          </h3>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
          >
            <X className="h-4 w-4" />
          </button>
        </header>

        {/* Form de creación: source (base) → target (dependiente). */}
        <div className="grid grid-cols-[1fr_auto_1fr_auto] items-center gap-2 border-b border-slate-800 px-5 py-3">
          <select
            value={source}
            onChange={(e) => setSource(e.target.value)}
            className="w-full rounded-md border border-slate-800 bg-slate-900 px-2 py-1.5 text-[11px] text-slate-200 focus:border-amber-500/40 focus:outline-none"
          >
            <option value="">{t("linkmap.modal.source_placeholder")}</option>
            {sorted.map(({ p }) => (
              <option key={p.folderName} value={p.folderName}>
                {p.title}
              </option>
            ))}
          </select>
          <span className="text-amber-400">→</span>
          <select
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            className="w-full rounded-md border border-slate-800 bg-slate-900 px-2 py-1.5 text-[11px] text-slate-200 focus:border-amber-500/40 focus:outline-none"
          >
            <option value="">{t("linkmap.modal.target_placeholder")}</option>
            {sorted.map(({ p }) => (
              <option key={p.folderName} value={p.folderName}>
                {p.title}
              </option>
            ))}
          </select>
          <button
            onClick={() => void addLink()}
            disabled={busy || !source || !target || source === target}
            className="inline-flex items-center gap-1 rounded-md bg-emerald-500 px-2.5 py-1.5 text-[11px] font-semibold text-emerald-950 hover:bg-emerald-400 disabled:opacity-50"
          >
            <Plus className="h-3 w-3" />
            {t("linkmap.modal.add")}
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-3">
          {links.length === 0 ? (
            <p className="py-6 text-center text-xs text-slate-500">
              {t("linkmap.modal.no_links")}
            </p>
          ) : (
            <ul className="space-y-1.5">
              {links.map((l) => (
                <li
                  key={l.id}
                  className="flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900/40 px-3 py-2"
                >
                  <span className="min-w-0 flex-1 truncate text-[11px] text-slate-200" title={l.sourceFolder}>
                    {titleOf(l.sourceFolder)}
                  </span>
                  <span className="shrink-0 text-amber-400">→</span>
                  <span className="min-w-0 flex-1 truncate text-[11px] text-slate-200" title={l.targetFolder}>
                    {titleOf(l.targetFolder)}
                  </span>
                  <span
                    className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide ${
                      l.origin === "auto"
                        ? "bg-sky-500/15 text-sky-300 ring-1 ring-sky-500/30"
                        : "bg-violet-500/15 text-violet-300 ring-1 ring-violet-500/30"
                    }`}
                  >
                    {l.origin === "auto"
                      ? t("linkmap.modal.origin_auto")
                      : t("linkmap.modal.origin_manual")}
                  </span>
                  <button
                    onClick={() => void removeLink(l)}
                    disabled={busy}
                    title={t("linkmap.modal.delete")}
                    className="shrink-0 rounded p-1 text-slate-500 hover:bg-rose-500/15 hover:text-rose-300 disabled:opacity-50"
                  >
                    <Trash2 className="h-3 w-3" />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** (v4.28.0) Clasifica un nodo por marca para el layout del Link Map.
 *  - Airbus → bucket izquierdo
 *  - Boeing → bucket derecho
 *  - Otros aviones (CRJ, ATR, Embraer, MD, Cessna…) → bucket central
 *  - Resto (sound packs, utilities, mods sin avión) → bucket "misc"
 *    abajo. */
// --- Zona de aeropuertos (v5.0.0) -----------------------------------------
// Clusters compactos por CONTINENTE en vez de una columna por región (que
// generaba columnas larguísimas con 271 addons). Cada cluster es una
// grilla de pocas columnas con una cabecera de rótulo, apilados
// verticalmente y desplazados a la derecha para aislarse de los aviones.

const AP_COL_W = 200; // 180 (card) + gap
const AP_ROW_H = 104;
const AP_HEADER_H = 54;
const AP_CLUSTER_GAP = 76;
// X base de la zona de aeropuertos: a la derecha de los 3 buckets de
// aviones (Airbus|Otros|Boeing ≈ 2550) + margen amplio de separación.
const AIRPORT_X_BASE = 3200;

// Orden de continentes (oeste→este aprox.) para apilar los clusters.
const CONTINENT_ORDER: Continent[] = [
  "north_america",
  "central_america",
  "caribbean",
  "south_america",
  "europe",
  "africa",
  "middle_east",
  "asia",
  "oceania",
  "other",
];

interface AirportZoneLayout {
  positions: Map<string, { x: number; y: number }>;
  headers: { id: string; label: string; count: number; x: number; y: number }[];
}

/** (v5.0.0) Reparte los aeropuertos en clusters por continente: cada
 *  cluster es una grilla de pocas columnas con su cabecera. Devuelve las
 *  posiciones + descriptores de cabecera. Standalone, sin relación con
 *  los addons. */
function layoutAirportZone(airports: CommunityPackage[]): AirportZoneLayout {
  const positions = new Map<string, { x: number; y: number }>();
  const headers: AirportZoneLayout["headers"] = [];

  const byCont = new Map<Continent, CommunityPackage[]>();
  for (const ap of airports) {
    const cont = airportRegion(ap.icao).continent;
    if (!byCont.has(cont)) byCont.set(cont, []);
    byCont.get(cont)!.push(ap);
  }

  let y = 0;
  for (const cont of CONTINENT_ORDER) {
    const list = byCont.get(cont);
    if (!list || list.length === 0) continue;
    list.sort((a, b) => (a.icao ?? "").localeCompare(b.icao ?? ""));
    // Pocas columnas → clusters cuadrados/compactos, no tiras largas.
    const cols = Math.min(6, Math.max(3, Math.ceil(Math.sqrt(list.length))));
    headers.push({
      id: `hdr:${cont}`,
      label: continentLabel(cont),
      count: list.length,
      x: AIRPORT_X_BASE,
      y,
    });
    const gridTop = y + AP_HEADER_H;
    list.forEach((ap, i) => {
      positions.set(ap.folderName, {
        x: AIRPORT_X_BASE + (i % cols) * AP_COL_W,
        y: gridTop + Math.floor(i / cols) * AP_ROW_H,
      });
    });
    const rows = Math.ceil(list.length / cols);
    y = gridTop + rows * AP_ROW_H + AP_CLUSTER_GAP;
  }
  return { positions, headers };
}

function brandOf(item: AddonItem): "airbus" | "boeing" | "other" | "misc" {
  const hay = `${item.p.title} ${item.p.folderName}`.toLowerCase();
  // Airbus: A3xx + A32NX/A380X.
  if (/\b(airbus|a3(?:1[89]|2[01]|30|40|50|80)|a32nx|a380x)\b/.test(hay)) {
    return "airbus";
  }
  // Boeing: 7xx + dreamliner + B7xx.
  if (/\b(boeing|7[3-9][0-9](?:[\s-]?(?:max|er|lr|f))?|b73[6-9]|b74[78]|b75[7]|b76[7]|b77[7]|b78[7]|dreamliner|salty\s*747)\b/.test(hay)) {
    return "boeing";
  }
  // Otros aviones reales.
  if (
    item.t === "AIRCRAFT" ||
    item.t === "LIVERY" ||
    /\b(crj|atr|embraer|md-?\d+|cessna|piper|king\s*air|tbm|c1?7[2358]|c20[8]|dh[c]?-?8|q400|cj4|airliner|aircraft|airplane|jet)\b/.test(hay)
  ) {
    return "other";
  }
  return "misc";
}

/** Layout en 4 buckets verticales (Airbus | Otros | Boeing | Misc).
 *  Cada bucket organiza internamente sus nodos con dagre LR — así un
 *  avión + sus liveries siguen apareciendo como cluster horizontal,
 *  pero distintos aviones se apilan verticalmente DENTRO de su marca.
 *  Buckets dispuestos lado a lado con offsets X grandes para que no
 *  se solapen. */
function layoutWithDagre(
  unpositioned: string[],
  allMembers: string[],
  links: AddonLink[],
  byFolder: Map<string, AddonItem>,
): Map<string, { x: number; y: number }> {
  const out = new Map<string, { x: number; y: number }>();
  if (unpositioned.length === 0) return out;

  // Para evaluar la marca de un livery enlazado, usamos la marca del
  // padre (el aircraft base) — así "PMDG 777 (PMDG772)" va al lado
  // derecho con su avión Boeing.
  const parentOf: Map<string, string> = new Map();
  for (const l of links) {
    parentOf.set(l.targetFolder, l.sourceFolder);
  }
  const brandFor = (folder: string): "airbus" | "boeing" | "other" | "misc" => {
    const item = byFolder.get(folder);
    if (!item) return "misc";
    const direct = brandOf(item);
    if (direct !== "misc" && direct !== "other") return direct;
    // Si el direct es "other" o "misc" pero el padre es Boeing/Airbus,
    // hereda la marca del padre.
    const parent = parentOf.get(folder);
    if (parent) {
      const parentItem = byFolder.get(parent);
      if (parentItem) {
        const parentBrand = brandOf(parentItem);
        if (parentBrand !== "misc") return parentBrand;
      }
    }
    return direct;
  };

  // Buckets disjuntos por marca.
  const buckets: Record<string, string[]> = {
    airbus: [],
    other: [],
    boeing: [],
    misc: [],
  };
  for (const m of allMembers) {
    buckets[brandFor(m)].push(m);
  }

  // X base de cada bucket (gap horizontal generoso para que liveries
  // de Boeing no choquen con las de Airbus).
  const BUCKET_W = 850;
  const offsets: Record<string, number> = {
    airbus: 0,
    other: BUCKET_W,
    boeing: BUCKET_W * 2,
    misc: 0, // se coloca debajo de todos
  };

  let maxBucketY = 0;
  for (const bucket of ["airbus", "other", "boeing"] as const) {
    const folders = buckets[bucket];
    if (folders.length === 0) continue;

    const g = new dagre.graphlib.Graph();
    g.setGraph({ rankdir: "LR", nodesep: 26, ranksep: 90, marginx: 12, marginy: 12 });
    g.setDefaultEdgeLabel(() => ({}));
    const folderSet = new Set(folders);
    for (const f of folders) g.setNode(f, { width: NODE_W, height: NODE_H });
    for (const l of links) {
      if (folderSet.has(l.sourceFolder) && folderSet.has(l.targetFolder)) {
        g.setEdge(l.sourceFolder, l.targetFolder);
      }
    }
    dagre.layout(g);
    for (const f of folders) {
      const n = g.node(f);
      if (!n) continue;
      const x = offsets[bucket] + n.x - NODE_W / 2;
      const y = n.y - NODE_H / 2;
      maxBucketY = Math.max(maxBucketY, y + NODE_H);
      if (unpositioned.includes(f)) {
        out.set(f, { x, y });
      }
    }
  }

  // Bucket misc: grilla ancha debajo de los 3 buckets de aviones.
  const misc = buckets.misc;
  if (misc.length > 0) {
    const cols = Math.max(8, Math.min(24, Math.ceil(Math.sqrt(misc.length * 4))));
    const gapX = NODE_W + 26;
    const gapY = NODE_H + 26;
    const startY = maxBucketY + 120;
    misc.forEach((m, i) => {
      if (unpositioned.includes(m)) {
        out.set(m, {
          x: (i % cols) * gapX,
          y: startY + Math.floor(i / cols) * gapY,
        });
      }
    });
  }
  return out;
}

function nodeTypeIcon(t: DerivedType, className = "h-6 w-6"): React.ReactNode {
  switch (t) {
    case "AIRCRAFT":
      return <Plane className={className} />;
    case "LIVERY":
      return <Palette className={className} />;
    case "INSTRUMENT":
      return <Cog className={className} />;
    case "MISC":
      return <Music className={className} />;
    default:
      return <HelpCircle className={className} />;
  }
}

function nodeBadgeLabel(t: DerivedType): string {
  switch (t) {
    case "AIRCRAFT":
      return "ACFT";
    case "LIVERY":
      return "LIVERY";
    case "INSTRUMENT":
      return "UTIL";
    case "MISC":
      return "MISC";
    default:
      return "?";
  }
}

function nodeBadgeClass(t: DerivedType): string {
  const base =
    "absolute left-1.5 top-1.5 rounded px-1 py-0.5 text-[8px] font-bold uppercase tracking-wide backdrop-blur";
  switch (t) {
    case "AIRCRAFT":
      return `${base} bg-violet-500/80 text-violet-50`;
    case "LIVERY":
      return `${base} bg-sky-500/80 text-sky-50`;
    case "INSTRUMENT":
      return `${base} bg-fuchsia-500/80 text-fuchsia-50`;
    case "MISC":
      return `${base} bg-amber-500/80 text-amber-950`;
    default:
      return `${base} bg-slate-700/80 text-slate-200`;
  }
}

