import { describe, expect, it } from "vitest";

import { facetId, interpret } from "./interpret";

const GENRES = ["Comedy", "Drama", "Science Fiction", "Fiction", "Crime"];
const summary = (q: string, dismissed?: Set<string>) => {
  const r = interpret(q, GENRES, dismissed);
  return { text: r.text, facets: r.facets.map((f) => facetId(f)) };
};

describe("interpret", () => {
  it("leaves unsupported extra filters in the searchable text", () => {
    const multiple = summary("crime drama");
    expect(multiple.facets.filter((f) => f.startsWith("genre:"))).toHaveLength(1);
    expect(["crime", "drama"]).toContain(multiple.text);
    expect(summary("movies 90s 2001")).toEqual({
      text: "2001",
      facets: ["type:movies", "decade:1990"],
    });
  });

  it("reads decades, genres and types, and searches what's left", () => {
    expect(summary("french comedies from the 90s")).toEqual({
      text: "french",
      facets: ["genre:Comedy", "decade:1990"],
    });
    expect(summary("sci-fi movies 2001")).toEqual({
      text: "",
      facets: ["genre:Science Fiction", "type:movies", "year:2001"],
    });
    expect(summary("crime shows of the 2010s")).toEqual({
      text: "",
      facets: ["genre:Crime", "type:series", "decade:2010"],
    });
    expect(summary("'80s science fiction")).toEqual({
      text: "",
      facets: ["decade:1980", "genre:Science Fiction"],
    });
  });

  it("leaves plain titles alone", () => {
    expect(summary("the wire")).toEqual({ text: "the wire", facets: [] });
    expect(summary("blade runner 2049")).toEqual({ text: "blade runner 2049", facets: [] });
    expect(summary("1917")).toEqual({ text: "1917", facets: [] });
    expect(summary("heist from 1995")).toEqual({ text: "heist", facets: ["year:1995"] });
  });

  it("searches a dismissed facet as words again", () => {
    expect(summary("drama 1999", new Set(["year:1999"]))).toEqual({
      text: "1999",
      facets: ["genre:Drama"],
    });
  });
});
