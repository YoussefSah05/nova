import { expect, it } from "vitest";
import { landscapeRowHeight } from "./gridLayout";

it("estimates a landscape row from the card width, excluding gaps", () => {
  // 4 cards across 1,300 px with 20 px gaps: 310 px cards, 174.4 px artwork.
  expect(landscapeRowHeight(1300, 4, 20)).toBe(Math.round(174.375 + 10 + 16 + 20));
  // One column: the card spans the full width.
  expect(landscapeRowHeight(320, 1, 20)).toBe(Math.round(180 + 10 + 16 + 20));
});
