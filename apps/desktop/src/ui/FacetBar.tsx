//! A section's filter bar: search, the facet menus, and the applied facets
//! as removable chips. Sticky under the top bar on a near-opaque surface: the
//! top bar is the only live blur, since each blurred layer is recomputed on
//! every scroll frame.

import { memo } from "react";
import { SearchIcon, XIcon } from "lucide-react";

import { SimpleSelect } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { FACET_LABELS, type Facet, type FacetKind, type FacetValues } from "./facets";

const ANY = "__any__";

export interface FacetMenu {
  kind: Exclude<FacetKind, "query">;
  options: string[];
  anyLabel: string;
}

export const FacetBar = memo(function FacetBar({
  searchLabel,
  values,
  menus,
  facets,
  onChange,
  onClear,
}: {
  searchLabel: string;
  values: FacetValues;
  menus: FacetMenu[];
  facets: Facet[];
  onChange: (kind: FacetKind, value: string) => void;
  onClear: () => void;
}) {
  return (
    <div
      role="toolbar"
      aria-label="Filters"
      className="sticky top-[84px] z-20 flex min-h-14 flex-wrap items-center gap-2 rounded-2xl border border-white/[0.08] bg-[#0e0e12]/95 px-2.5 py-2 shadow-[0_8px_24px_rgba(0,0,0,0.45)]"
    >
      <label className="flex h-9 w-64 items-center gap-2 rounded-[10px] border border-white/10 bg-white/[0.05] px-2.5 text-white/60 focus-within:border-white/25">
        <SearchIcon className="h-4 w-4 shrink-0" strokeWidth={1.5} aria-hidden />
        <input
          type="search"
          value={values.query ?? ""}
          onChange={(e) => onChange("query", e.target.value)}
          placeholder={searchLabel}
          aria-label={searchLabel}
          className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-white/45"
        />
      </label>
      {menus
        .filter((m) => m.options.length > 0)
        .map((m) => (
          <SimpleSelect
            key={m.kind}
            value={values[m.kind] || ANY}
            options={[
              { value: ANY, label: m.anyLabel },
              ...m.options.map((o) => ({ value: o, label: o })),
            ]}
            onChange={(v) => onChange(m.kind, v === ANY ? "" : v)}
            ariaLabel={FACET_LABELS[m.kind]}
            className="h-9 min-w-[150px] rounded-[10px] border-white/10 bg-white/[0.04]"
          />
        ))}
      {facets.length > 0 && <span aria-hidden className="mx-1 h-6 w-px bg-white/[0.12]" />}
      {facets.map((f) => (
        <button
          key={f.kind}
          type="button"
          onClick={() => onChange(f.kind, "")}
          aria-label={`Clear ${FACET_LABELS[f.kind]} filter`}
          className={cn(
            "flex h-[30px] max-w-[16rem] items-center gap-1.5 rounded-full border pl-3 pr-2 text-xs font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            "border-primary/45 bg-primary/[0.12] text-[var(--color-brand-hover)] hover:bg-primary/20"
          )}
        >
          <span className="text-white/55">{FACET_LABELS[f.kind]}</span>
          <span className="truncate">{f.value}</span>
          <XIcon className="h-3 w-3 shrink-0" strokeWidth={1.5} aria-hidden />
        </button>
      ))}
      {facets.length > 1 && (
        <button
          type="button"
          onClick={onClear}
          className="h-[30px] rounded-full px-2 text-xs text-white/60 transition-colors hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          Clear all
        </button>
      )}
    </div>
  );
});
