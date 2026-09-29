//! Facets: the filters a browse view applies, as data. Chips in a section's
//! filter bar and (later) the typed intents Command-K understands share this
//! model, so an interpreted request and a hand-picked filter look and undo
//! the same way.

import type { BrowseFilters } from "@/features/browse/queries";

export type FacetKind =
  | "query"
  | "category"
  | "region"
  | "country"
  | "genre"
  | "decade"
  | "collection"
  | "status";

export interface Facet {
  kind: FacetKind;
  value: string;
  /// What the chip shows when the value is an id or a code (`1990` → `1990s`).
  label?: string;
  /// Who applied it: the user in a filter bar, or an interpreted request.
  origin: "user" | "intent";
}

export const FACET_LABELS: Record<FacetKind, string> = {
  query: "Search",
  category: "Genre",
  region: "Region",
  country: "Country",
  genre: "Genre",
  decade: "Decade",
  collection: "Collection",
  status: "Status",
};

export type FacetValues = Partial<Record<FacetKind, string>>;

/// The facets in effect (empty values dropped), in a stable order.
export function activeFacets(values: FacetValues, origin: Facet["origin"] = "user"): Facet[] {
  return (["query", "category", "region", "country"] as const)
    .map((kind) => ({ kind, value: (values[kind] ?? "").trim(), origin }))
    .filter((f) => f.value.length > 0);
}

/// Browse filters for the facets; live-only facets are dropped elsewhere.
export function facetFilters(facets: Facet[], live: boolean): BrowseFilters {
  const get = (kind: FacetKind) => facets.find((f) => f.kind === kind)?.value ?? null;
  return {
    query: get("query"),
    category: get("category"),
    country: live ? get("country") : null,
    region: live ? get("region") : null,
  };
}
