import { Search, Loader2 } from "lucide-react";
import { useEffect, useRef } from "react";
import { cn } from "../lib/cn";
import { t } from "../lib/i18n";

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
  placeholder,
}: Props) {
  const effectivePlaceholder = placeholder ?? t("search.placeholder.scenery");
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
          "flex w-full items-center gap-3 rounded-xl border border-sf-border bg-sf-surface px-4 py-2.5 transition-all",
          loading
            ? "cursor-wait opacity-60"
            : "focus-within:border-brand-500/60 focus-within:ring-2 focus-within:ring-brand-500/20",
        )}
      >
        {loading ? (
          <Loader2 className="h-5 w-5 shrink-0 animate-spin text-brand-400" />
        ) : (
          <Search className="h-5 w-5 shrink-0 text-sf-muted" />
        )}
        <input
          ref={ref}
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={loading ? t("search.searching") : effectivePlaceholder}
          disabled={loading}
          aria-disabled={loading}
          // `readOnly` on top of `disabled` keeps the element keyboard-
          // focusable on re-enable (browsers skip disabled fields when
          // tabbing), while `disabled` still blocks input while loading.
          className="w-full bg-transparent text-[15px] text-sf-text placeholder:text-sf-muted/80 focus:outline-none disabled:cursor-wait disabled:text-sf-muted"
        />
        <kbd
          className={cn(
            "hidden rounded border border-sf-border bg-sf-bg px-1.5 py-0.5 text-[11px] text-sf-muted md:inline",
            loading && "opacity-50",
          )}
        >
          Enter
        </kbd>
      </div>
    </form>
  );
}
