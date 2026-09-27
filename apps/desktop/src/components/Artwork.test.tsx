import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

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

  describe("inside Tauri", () => {
    beforeEach(() => {
      Object.defineProperty(window, "__TAURI_INTERNALS__", {
        configurable: true,
        value: {
          convertFileSrc: (path: string, protocol: string) =>
            `${protocol}://localhost/${encodeURIComponent(path)}`,
        },
      });
    });
    afterEach(() => {
      delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    });

    const url = "https://x.tv/logo.png";
    const cached = `nova-art://localhost/${encodeURIComponent(url)}`;

    it("loads through the artwork cache, then the original, then the fallback", () => {
      render(<Artwork src={url} alt="logo" fallback={fallback} />);
      const img = screen.getByAltText("logo");
      expect(img).toHaveAttribute("src", cached);
      fireEvent.error(img);
      expect(screen.getByAltText("logo")).toHaveAttribute("src", url);
      fireEvent.error(screen.getByAltText("logo"));
      expect(screen.getByText("fallback")).toBeInTheDocument();
    });

    it("skips the cache route for a URL it already failed", () => {
      const { unmount } = render(<Artwork src={url} alt="logo" fallback={fallback} />);
      fireEvent.error(screen.getByAltText("logo"));
      unmount();
      render(<Artwork src={url} alt="logo" fallback={fallback} />);
      expect(screen.getByAltText("logo")).toHaveAttribute("src", url);
    });

    it("leaves non-http URLs alone", () => {
      render(<Artwork src="data:image/png;base64,AAAA" alt="logo" />);
      expect(screen.getByAltText("logo")).toHaveAttribute("src", "data:image/png;base64,AAAA");
    });
  });
});
