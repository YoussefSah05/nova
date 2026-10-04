import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DecadesShelf, GenresShelf } from "./Shelves";

describe("Movies shelves while loading", () => {
  it("hold their place with a placeholder until data arrives", () => {
    const { container } = render(
      <>
        <DecadesShelf decades={undefined} loading active={null} onPick={vi.fn()} />
        <GenresShelf genres={undefined} loading active={null} onPick={vi.fn()} />
      </>
    );
    expect(screen.getByRole("heading", { name: "By decade" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "By genre" })).toBeInTheDocument();
    expect(container.querySelectorAll(".nova-shimmer").length).toBeGreaterThan(0);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("disappear when the library has none, and show real entries when it does", () => {
    const { rerender } = render(<GenresShelf genres={[]} active={null} onPick={vi.fn()} />);
    expect(screen.queryByRole("heading", { name: "By genre" })).not.toBeInTheDocument();
    rerender(
      <GenresShelf genres={[{ name: "Drama", count: 12 }]} active={null} onPick={vi.fn()} />
    );
    expect(screen.getByRole("button", { name: /Drama/ })).toBeInTheDocument();
  });

  it("show nothing when the read failed instead of shimmering forever", () => {
    const { container } = render(
      <GenresShelf genres={undefined} loading={false} active={null} onPick={vi.fn()} />
    );
    expect(container).toBeEmptyDOMElement();
  });
});
