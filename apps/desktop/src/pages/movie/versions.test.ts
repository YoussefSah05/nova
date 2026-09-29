import { describe, expect, it } from "vitest";

import type { VariantSummary } from "@/types";
import { orderVersions, versionReason } from "./versions";

function v(id: string, extra: Partial<VariantSummary> = {}): VariantSummary {
  return {
    variantId: id,
    sourceId: 1,
    sourceName: `Source ${id}`,
    label: id,
    available: true,
    badges: [],
    recentlyFailed: false,
    picked: false,
    ...extra,
  };
}

describe("film versions", () => {
  it("puts Nova's pick first and unplayable sources last", () => {
    const ordered = orderVersions([
      v("gone", { available: false }),
      v("failed", { recentlyFailed: true }),
      v("other"),
      v("pick", { picked: true }),
    ]);
    expect(ordered.map((x) => x.variantId)).toEqual(["pick", "other", "failed", "gone"]);
  });

  it("explains each source from its evidence, never beyond it", () => {
    expect(versionReason(v("a", { picked: true, badges: ["FHD", "VF"] }), "fr")).toBe(
      "Nova plays this one: FHD with French audio."
    );
    // No audio evidence on the pick: the reason stays about the picture.
    expect(versionReason(v("b", { picked: true, badges: ["FHD"] }), "fr")).toBe(
      "Nova plays this one: full HD, the steadiest picture over IPTV."
    );
    expect(versionReason(v("c", { badges: ["4K"] }), null)).toBe(
      "4K streams stall more often over IPTV."
    );
    expect(versionReason(v("d", { recentlyFailed: true, badges: ["FHD"] }), null)).toBe(
      "Failed recently — tried last."
    );
    expect(versionReason(v("e", { available: false }), null)).toBe(
      "No longer listed by the provider."
    );
  });
});
