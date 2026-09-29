//! What Nova understood in a search, as removable chips: "Comedy", "1990s",
//! "Shows". Removing one searches its words as plain text again. Derived,
//! so neutral and dashed — never amber, which marks the user's own choices.

import { memo } from "react";
import { SparklesIcon, XIcon } from "lucide-react";

import { FACET_LABELS, type Facet } from "./facets";
import { facetId } from "./interpret";

export const InterpretationChips = memo(function InterpretationChips({
  facets,
  onDismiss,
}: {
  facets: Facet[];
  onDismiss: (facet: Facet) => void;
}) {
  if (facets.length === 0) return null;
  return (
    <div role="group" aria-label="Understood as" className="flex flex-wrap items-center gap-2">
      <span className="flex items-center gap-1.5 text-xs text-white/55">
        <SparklesIcon className="h-3.5 w-3.5" strokeWidth={1.5} aria-hidden />
        Understood as
      </span>
      {facets.map((f) => {
        const label = f.label ?? f.value;
        return (
          <span
            key={facetId(f)}
            className="flex h-7 items-center gap-1 rounded-full border border-dashed border-white/25 pl-3 pr-1 text-xs text-white/80"
          >
            <span className="text-white/50">{FACET_LABELS[f.kind]}</span>
            <span>{label}</span>
            <button
              type="button"
              onClick={() => onDismiss(f)}
              aria-label={`Search “${label}” as words instead`}
              className="ml-0.5 flex h-5 w-5 items-center justify-center rounded-full text-white/60 hover:bg-white/10 hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            >
              <XIcon className="h-3 w-3" strokeWidth={1.5} aria-hidden />
            </button>
          </span>
        );
      })}
    </div>
  );
});
