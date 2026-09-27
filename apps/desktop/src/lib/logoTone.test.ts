import { describe, expect, it } from "vitest";

import { NEUTRAL_TONE, analyzeLogoPixels } from "./logoTone";

const W = 20;
const H = 20;
type Px = [number, number, number, number];

/** An image whose pixel at (x, y) is `paint(x, y)`. */
function image(paint: (x: number, y: number) => Px): Uint8ClampedArray {
  const data = new Uint8ClampedArray(W * H * 4);
  for (let y = 0; y < H; y++) {
    for (let x = 0; x < W; x++) data.set(paint(x, y), (y * W + x) * 4);
  }
  return data;
}

const inner = (x: number, y: number) => x >= 5 && x < 15 && y >= 6 && y < 14;
const CLEAR: Px = [0, 0, 0, 0];

describe("analyzeLogoPixels", () => {
  it("fills the tile with the edge colour of a boxed logo", () => {
    // A white JPEG box with a red wordmark.
    const tone = analyzeLogoPixels(
      image((x, y) => (inner(x, y) ? [200, 20, 30, 255] : [255, 255, 255, 255])),
      W,
      H
    );
    expect(tone).toEqual({ mode: "fill", background: "rgb(255, 255, 255)" });
  });

  it("covers the tile with a busy opaque picture", () => {
    const tone = analyzeLogoPixels(
      image((x, y) => ((x + y) % 2 ? [240, 240, 240, 255] : [10, 60, 120, 255])),
      W,
      H
    );
    expect(tone.mode).toBe("cover");
  });

  it("tints a transparent bright logo with its own hue", () => {
    const tone = analyzeLogoPixels(
      image((x, y) => (inner(x, y) ? [255, 140, 0, 255] : CLEAR)),
      W,
      H
    );
    expect(tone.mode).toBe("tint");
    expect(tone.background).toMatch(/^linear-gradient\(135deg, hsl\(33 /);
  });

  it("gives deep-coloured ink a light tile in its hue (contrast first)", () => {
    const tone = analyzeLogoPixels(
      image((x, y) => (inner(x, y) ? [0, 60, 190, 255] : CLEAR)),
      W,
      H
    );
    expect(tone.mode).toBe("light");
    expect(tone.background).toMatch(/hsl\(22\d 28% 96%\)/);
  });

  it("puts dark ink on a light tile", () => {
    const tone = analyzeLogoPixels(
      image((x, y) => (inner(x, y) ? [15, 15, 20, 255] : CLEAR)),
      W,
      H
    );
    expect(tone.mode).toBe("light");
    expect(tone.background).toContain("96%");
  });

  it("uses the neutral slate for white logos and near-empty images", () => {
    const white = analyzeLogoPixels(
      image((x, y) => (inner(x, y) ? [250, 250, 250, 255] : CLEAR)),
      W,
      H
    );
    expect(white).toEqual(NEUTRAL_TONE);
    expect(
      analyzeLogoPixels(
        image(() => CLEAR),
        W,
        H
      )
    ).toEqual(NEUTRAL_TONE);
  });
});
