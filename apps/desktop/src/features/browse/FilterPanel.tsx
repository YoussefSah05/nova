//! Filters popover behind the toolbar's "Filters" button: region + country
//! (live only) and a genre pill grid, on glass. Selecting updates the same
//! filter state the grid reads; active filters also surface as removable chips
//! in the toolbar (see Home).

import { SlidersHorizontal } from "lucide-react";

import { cn } from "@/lib/utils";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Button } from "@/components/ui/button";
import { SimpleSelect } from "@/components/ui/select";
import type { BrowseTab } from "./queries";

const ALL = "__all__";

export function FilterPanel({
  tab,
  regionFilter,
  setRegionFilter,
  regionOptions,
  countryFilter,
  setCountryFilter,
  countryOptions,
  categoryFilter,
  setCategoryFilter,
  categoryOptions,
  activeCount,
}: {
  tab: BrowseTab;
  regionFilter: string;
  setRegionFilter: (v: string) => void;
  regionOptions: string[];
  countryFilter: string;
  setCountryFilter: (v: string) => void;
  countryOptions: string[];
  categoryFilter: string;
  setCategoryFilter: (v: string) => void;
  categoryOptions: string[];
  activeCount: number;
}) {
  const live = tab === "live";
  const hasAnything =
    (live && (regionOptions.length > 0 || countryOptions.length > 0)) || categoryOptions.length > 0;

  const selectOptions = (opts: string[], allLabel: string) => [
    { value: ALL, label: allLabel },
    ...opts.map((o) => ({ value: o, label: o })),
  ];

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="secondary" className="gap-2">
          <SlidersHorizontal className="h-4 w-4" aria-hidden />
          Filters
          {activeCount > 0 && (
            <span className="flex h-5 min-w-5 items-center justify-center rounded-full bg-primary px-1 text-[11px] font-semibold text-primary-foreground tabular-nums">
              {activeCount}
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80">
        <div className="flex flex-col gap-4">
          {live && regionOptions.length > 0 && (
            <Field label="Region">
              <SimpleSelect
                value={regionFilter || ALL}
                options={selectOptions(regionOptions, "All regions")}
                onChange={(v) => setRegionFilter(v === ALL ? "" : v)}
                ariaLabel="Region"
                className="w-full"
              />
            </Field>
          )}
          {live && (
            <Field label="Country">
              <SimpleSelect
                value={countryFilter || ALL}
                options={selectOptions(countryOptions, "All countries")}
                onChange={(v) => setCountryFilter(v === ALL ? "" : v)}
                ariaLabel="Country"
                className="w-full"
              />
            </Field>
          )}

          {categoryOptions.length > 0 && (
            <Field label="Genre">
              <div className="-mr-1 flex max-h-56 flex-wrap gap-1.5 overflow-y-auto pr-1 [scrollbar-width:thin]">
                <GenrePill active={categoryFilter === ""} onClick={() => setCategoryFilter("")}>
                  All
                </GenrePill>
                {categoryOptions.map((option) => (
                  <GenrePill
                    key={option}
                    active={categoryFilter === option}
                    onClick={() => setCategoryFilter(categoryFilter === option ? "" : option)}
                  >
                    {option}
                  </GenrePill>
                ))}
              </div>
            </Field>
          )}

          {!hasAnything && (
            <p className="py-2 text-center text-xs text-white/40">No filters for this section.</p>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <p className="text-[11px] font-semibold uppercase tracking-wider text-white/45">{label}</p>
      {children}
    </div>
  );
}

function GenrePill({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "shrink-0 rounded-full border px-3 py-1 text-xs font-medium transition-[color,background-color,box-shadow,transform] active:scale-95 focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active
          ? "border-transparent bg-primary text-primary-foreground shadow-sm"
          : "border-white/10 bg-white/[0.04] text-white/80 hover:border-white/20 hover:bg-white/[0.08]"
      )}
    >
      {children}
    </button>
  );
}
