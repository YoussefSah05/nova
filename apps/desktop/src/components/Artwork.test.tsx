import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { Artwork } from "@/components/Artwork";
import { resetBrokenArtwork } from "@/lib/artwork";

const fallback = <span>fallback</span>;

describe("Artwork", () => {
  beforeEach(() => resetBrokenArtwork());

  it("renders the image when a URL is given", () => {
    render(<Artwork src="https://x.tv/logo.png" alt="logo" fallback={fallback} />);
    expect(screen.getByAltText("logo")).toHaveAttribute("src", "https://x.tv/logo.png");
    expect(screen.queryByText("fallback")).toBeNull();
  });

  it("renders the fallback without a URL", () => {
    render(<Artwork src={null} fallback={fallback} />);
    expect(screen.getByText("fallback")).toBeInTheDocument();
  });

  it("swaps a broken image for the fallback and does not retry it on remount", () => {
    const { unmount } = render(
      <Artwork src="https://x.tv/dead.png" alt="logo" fallback={fallback} />
    );
    fireEvent.error(screen.getByAltText("logo"));
    expect(screen.getByText("fallback")).toBeInTheDocument();
    unmount();

    render(<Artwork src="https://x.tv/dead.png" alt="logo" fallback={fallback} />);
    expect(screen.queryByAltText("logo")).toBeNull();
    expect(screen.getByText("fallback")).toBeInTheDocument();
  });

  it("tries a new URL after the previous one failed", () => {
    const { rerender } = render(
      <Artwork src="https://x.tv/dead.png" alt="logo" fallback={fallback} />
    );
    fireEvent.error(screen.getByAltText("logo"));
    rerender(<Artwork src="https://x.tv/ok.png" alt="logo" fallback={fallback} />);
    expect(screen.getByAltText("logo")).toHaveAttribute("src", "https://x.tv/ok.png");
  });
});
