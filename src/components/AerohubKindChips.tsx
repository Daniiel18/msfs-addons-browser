import { Cog, Fan, MapPin, Package, Plane } from "lucide-react";
import { useAppStore } from "../stores/useAppStore";
import { TypeChip } from "./TypeChip";
import { t } from "../lib/i18n";

/** (26/10) Tipos de la web de AeroHub (`/browse?kind=`), en su orden. */
const KINDS: { kind: string | null; icon: React.ReactNode; label: string }[] = [
  { kind: null, icon: <Package className="h-3.5 w-3.5" />, label: "explore.kind.all" },
  { kind: "plane", icon: <Plane className="h-3.5 w-3.5" />, label: "explore.kind.plane" },
  { kind: "helicopter", icon: <Fan className="h-3.5 w-3.5" />, label: "explore.kind.helicopter" },
  { kind: "scenery", icon: <MapPin className="h-3.5 w-3.5" />, label: "explore.kind.scenery" },
  { kind: "utility", icon: <Cog className="h-3.5 w-3.5" />, label: "explore.kind.utility" },
];

/**
 * (26/10) Filtros por tipo del catálogo de AeroHub, con las mismas píldoras
 * que la página de Addons. Valen al explorar y al buscar; el simulador no
 * hace falta (SimFleet ya sólo enseña lo del MSFS elegido).
 */
export function AerohubKindChips() {
  const kind = useAppStore((s) => s.browseKind);
  const setKind = useAppStore((s) => s.setBrowseKind);
  return (
    <div className="flex flex-wrap gap-1.5">
      {KINDS.map((k) => (
        <TypeChip
          key={k.label}
          active={kind === k.kind}
          onClick={() => {
            if (kind !== k.kind) void setKind(k.kind);
          }}
          icon={k.icon}
          label={t(k.label)}
        />
      ))}
    </div>
  );
}
