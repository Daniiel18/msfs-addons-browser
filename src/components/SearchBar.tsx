import { Search, Loader2 } from "lucide-react";
import { useEffect, useRef } from "react";
import { cn } from "../lib/cn";

interface Props {
  value: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  loading?: boolean;
  placeholder?: string;
}

export function SearchBar({
  value,
  onChange,
  onSubmit,
  loading = false,
  placeholder = "Busca por aeropuerto, ICAO o desarrollador…",
}: Props) {
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    ref.current?.focus();
  }, []);

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        // Guard at the boundary too: the outer handler also early-returns
        // when a search is already in flight, but form submit can also be
        // triggered from keyboard navigation/password managers, so we
        // belt-and-suspenders it here.
        if (loading) return;
        onSubmit();
      }}
      className="group relative w-full"
      aria-busy={loading}
    >
      <div
        className={cn(
          "flex w-full items-center gap-3 rounded-2xl border border-slate-800 bg-slate-900/60 px-4 py-3 backdrop-blur transition-all",
          loading
            ? "cursor-wait opacity-60"
            : "focus-within:border-brand-500/60 focus-within:ring-2 focus-within:ring-brand-500/20",
        )}
      >
        {loading ? (
          <Loader2 className="h-5 w-5 shrink-0 animate-spin text-brand-400" />
        ) : (
          <Search className="h-5 w-5 shrink-0 text-slate-400" />
        )}
        <input
          ref={ref}
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={loading ? "Buscando…" : placeholder}
          disabled={loading}
          aria-disabled={loading}
          // `readOnly` on top of `disabled` keeps the element keyboard-
          // focusable on re-enable (browsers skip disabled fields when
          // tabbing), while `disabled` still blocks input while loading.
          className="w-full bg-transparent text-base text-slate-100 placeholder:text-slate-500 focus:outline-none disabled:cursor-wait disabled:text-slate-400"
        />
        <kbd
          className={cn(
            "hidden rounded border border-slate-700 bg-slate-800 px-1.5 py-0.5 text-xs text-slate-400 md:inline",
            loading && "opacity-50",
          )}
        >
          Enter
        </kbd>
      </div>
    </form>
  );
}
