/**
 * (v4.25.0) Píldora de filtro por tipo (página de Addons). (26/10) Compartida
 * con los filtros de AeroHub en Explorar; el contador es opcional.
 */
export function TypeChip({
  active,
  onClick,
  icon,
  label,
  count,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
  count?: number;
}) {
  return (
    <button
      onClick={onClick}
      className={`inline-flex items-center gap-1.5 rounded-full border px-3 py-1 text-xs transition-colors ${
        active
          ? "border-brand-500 bg-brand-500/15 text-brand-100"
          : "border-sf-border bg-sf-surface text-sf-text/90 hover:border-sf-border hover:bg-sf-raised"
      }`}
    >
      {icon}
      {label}
      {count !== undefined && (
        <span
          className={`ml-0.5 rounded-full px-1.5 text-[10px] font-medium ${
            active ? "bg-brand-500/30 text-brand-100" : "bg-sf-raised text-sf-muted"
          }`}
        >
          {count}
        </span>
      )}
    </button>
  );
}
