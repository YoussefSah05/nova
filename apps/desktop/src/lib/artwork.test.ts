import { describe, expect, it } from "vitest";

import { tmdbSized } from "./artwork";

describe("tmdbSized", () => {
  it("downsizes TMDB images and leaves everything else alone", () => {
    expect(tmdbSized("https://image.tmdb.org/t/p/w1280/a.jpg", "w780")).toBe(
      "https://image.tmdb.org/t/p/w780/a.jpg"
    );
    expect(tmdbSized("https://image.tmdb.org/t/p/original/a.jpg", "w780")).toBe(
      "https://image.tmdb.org/t/p/w780/a.jpg"
    );
    expect(tmdbSized("https://image.tmdb.org/t/p/w500/a.jpg", "w780")).toBe(
      "https://image.tmdb.org/t/p/w500/a.jpg"
    );
    expect(tmdbSized("http://panel/img/a.jpg", "w780")).toBe("http://panel/img/a.jpg");
    expect(tmdbSized(null, "w780")).toBeNull();
  });
});
