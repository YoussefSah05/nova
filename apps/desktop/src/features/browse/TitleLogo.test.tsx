import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { resetBrokenArtwork } from "@/lib/artwork";
import { TitleLogo } from "./TitleLogo";

describe("TitleLogo", () => {
  beforeEach(() => resetBrokenArtwork());

  it("shows the title logo inside the heading, named by the title", () => {
    render(<TitleLogo title="The Godfather" logoUrl="https://image.tmdb.org/t/p/w500/l.png" />);
    const heading = screen.getByRole("heading", { name: "The Godfather" });
    expect(heading.querySelector("img")).toHaveAttribute(
      "src",
      "https://image.tmdb.org/t/p/w500/l.png"
    );
  });

  it("falls back to the plain title without a logo or when it fails", () => {
    const { rerender } = render(<TitleLogo title="Dune" logoUrl={null} />);
    expect(screen.getByRole("heading", { name: "Dune" })).toHaveTextContent("Dune");

    rerender(<TitleLogo title="Dune" logoUrl="https://x.tv/dune.png" />);
    fireEvent.error(screen.getByAltText("Dune"));
    expect(screen.getByRole("heading", { name: "Dune" }).querySelector("img")).toBeNull();
    expect(screen.getByRole("heading")).toHaveTextContent("Dune");
  });
});
