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
  Link2,
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

const NODE_TYPES = { addon: AddonNode };

// ---------------------------------------------------------------------------
// Vista principal
// ---------------------------------------------------------------------------

export function LinkMapView({ addons }: { addons: AddonItem[] }) {
  const pushToast = useToastStore((s) => s.push);
  const [links, setLinks] = useState<AddonLink[]>([]);
  const [positions, setPositions] = useState<Map<string, AddonNodePosition>>(
    new Map(),
  );
  const [loaded, setLoaded] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  const [addQuery, setAddQuery] = useState("");
  const [findQuery, setFindQuery] = useState("");
  const flowRef = useRef<ReactFlowInstance<AddonNodeType, Edge> | null>(null);

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

  // Membresía del lienzo: extremos de links + posiciones guardadas.
  // Solo addons que EXISTEN en el inventario actual (byFolder).
  const members = useMemo(() => {
    const set = new Set<string>();
    for (const l of links) {
      if (byFolder.has(l.sourceFolder) && byFolder.has(l.targetFolder)) {
        set.add(l.sourceFolder);
        set.add(l.targetFolder);
      }
    }
    for (const f of positions.keys()) {
      if (byFolder.has(f)) set.add(f);
    }
    return Array.from(set);
  }, [links, positions, byFolder]);

  const visibleLinks = useMemo(
    () =>
      links.filter(
        (l) => byFolder.has(l.sourceFolder) && byFolder.has(l.targetFolder),
      ),
    [links, byFolder],
  );

  const [nodes, setNodes, onNodesChange] = useNodesState<AddonNodeType>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);

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
      );
      return members.map((folder) => {
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
    });
    setEdges(
      visibleLinks.map((l) => ({
        id: `${l.sourceFolder}->${l.targetFolder}`,
        source: l.sourceFolder,
        target: l.targetFolder,
        animated: true,
        style: { stroke: "#f59e0b", strokeWidth: 1.5, strokeDasharray: "7 5" },
        markerEnd: { type: MarkerType.ArrowClosed, color: "#f59e0b", width: 16, height: 16 },
      })),
    );
  }, [loaded, members, visibleLinks, byFolder, positions, setNodes, setEdges]);

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
    (_: unknown, node: AddonNodeType) => {
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

  // "Find addon on the map": centra la cámara en el primer match.
  const findOnMap = (q: string) => {
    setFindQuery(q);
    const needle = q.trim().toLowerCase();
    if (!needle) return;
    const hit = nodes.find(
      (n) =>
        n.data.pkg.title.toLowerCase().includes(needle) ||
        n.id.toLowerCase().includes(needle),
    );
    if (hit && flowRef.current) {
      flowRef.current.setCenter(
        hit.position.x + NODE_W / 2,
        hit.position.y + NODE_H / 2,
        { zoom: 1.1, duration: 500 },
      );
    }
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

      <div className="absolute left-1/2 top-3 z-10 w-64 -translate-x-1/2">
        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3 w-3 -translate-y-1/2 text-slate-500" />
          <input
            value={findQuery}
            onChange={(e) => findOnMap(e.target.value)}
            placeholder={t("linkmap.find_placeholder")}
            className="w-full rounded-lg border border-slate-800 bg-slate-950/80 py-1.5 pl-8 pr-2 text-[11px] text-slate-200 placeholder:text-slate-600 backdrop-blur focus:border-amber-500/40 focus:outline-none"
          />
        </div>
      </div>

      <div className="absolute right-3 top-3 z-10 flex items-center gap-2">
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

/** Auto-layout para los nodos sin posición persistida.
 *
 *  (v4.26.0) Dos zonas: los nodos CONECTADOS van con dagre (izquierda
 *  → derecha, clusters avión → liveries); los AISLADOS van en una
 *  GRILLA compacta debajo. Antes dagre metía a los aislados en un
 *  único rank → una columna vertical interminable, justo lo que el
 *  usuario reportó. */
function layoutWithDagre(
  unpositioned: string[],
  allMembers: string[],
  links: AddonLink[],
): Map<string, { x: number; y: number }> {
  const out = new Map<string, { x: number; y: number }>();
  if (unpositioned.length === 0) return out;

  const memberSet = new Set(allMembers);
  const linked = new Set<string>();
  for (const l of links) {
    if (memberSet.has(l.sourceFolder) && memberSet.has(l.targetFolder)) {
      linked.add(l.sourceFolder);
      linked.add(l.targetFolder);
    }
  }

  // Zona 1 — dagre solo con los nodos que participan en links.
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: "LR", nodesep: 36, ranksep: 110, marginx: 20, marginy: 20 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const m of allMembers) {
    if (linked.has(m)) {
      g.setNode(m, { width: NODE_W, height: NODE_H });
    }
  }
  for (const l of links) {
    if (linked.has(l.sourceFolder) && linked.has(l.targetFolder)) {
      g.setEdge(l.sourceFolder, l.targetFolder);
    }
  }
  let maxY = 0;
  if (linked.size > 0) {
    dagre.layout(g);
    for (const m of allMembers) {
      if (!linked.has(m)) continue;
      const n = g.node(m);
      if (n) {
        maxY = Math.max(maxY, n.y + NODE_H / 2);
        if (unpositioned.includes(m)) {
          out.set(m, { x: n.x - NODE_W / 2, y: n.y - NODE_H / 2 });
        }
      }
    }
  }

  // Zona 2 — grilla para los aislados, debajo de los clusters.
  // (v4.27.0) Más columnas (sqrt * 3 ≈ relación 16:9 visual) para
  // que las filas queden anchas y horizontales en vez de una columna
  // vertical infinita que el usuario reportó.
  const isolated = unpositioned.filter((m) => !linked.has(m));
  if (isolated.length > 0) {
    const cols = Math.max(
      6,
      Math.min(20, Math.ceil(Math.sqrt(isolated.length * 3))),
    );
    const gapX = NODE_W + 46;
    const gapY = NODE_H + 46;
    const startY = maxY > 0 ? maxY + 90 : 0;
    isolated.forEach((m, i) => {
      out.set(m, {
        x: (i % cols) * gapX,
        y: startY + Math.floor(i / cols) * gapY,
      });
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

