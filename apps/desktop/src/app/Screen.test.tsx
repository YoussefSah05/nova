import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Screen } from "./Screen";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

function mount(active: boolean) {
  return render(
    <Screen active={active} label="Movies" onScrolledChange={vi.fn()}>
      <div style={{ height: 4000 }}>content</div>
    </Screen>
  );
}

describe("Screen scrolling", () => {
  it("takes the pointer off its content while scrolling, and gives it back when idle", () => {
    mount(true);
    const region = screen.getByRole("region", { name: "Movies" });
    fireEvent.scroll(region);
    expect(region).toHaveClass("nova-scrolling");
    act(() => vi.advanceTimersByTime(100));
    fireEvent.scroll(region);
    act(() => vi.advanceTimersByTime(100));
    expect(region).toHaveClass("nova-scrolling");
    act(() => vi.advanceTimersByTime(60));
    expect(region).not.toHaveClass("nova-scrolling");
  });

  it("ignores scroll events while hidden", () => {
    const { container } = mount(false);
    // Inert while hidden, so it is out of the accessibility tree.
    const region = container.querySelector(".nova-screen")!;
    fireEvent.scroll(region);
    expect(region).not.toHaveClass("nova-scrolling");
  });

  it("gives the pointer back on a real pointer move, not a synthetic one", () => {
    mount(true);
    const region = screen.getByRole("region", { name: "Movies" });
    fireEvent.scroll(region);
    region.dispatchEvent(new MouseEvent("pointermove", { bubbles: true }));
    expect(region).toHaveClass("nova-scrolling");
    const move = new MouseEvent("pointermove", { bubbles: true });
    Object.defineProperty(move, "movementX", { value: 4 });
    region.dispatchEvent(move);
    expect(region).not.toHaveClass("nova-scrolling");
  });
});
