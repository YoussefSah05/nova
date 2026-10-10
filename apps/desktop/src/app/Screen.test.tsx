import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Screen } from "./Screen";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

it("protects horizontal rail scrolling while title activation remains available", () => {
  const activate = vi.fn();
  render(
    <Screen active label="Home" onScrolledChange={() => {}}>
      <div data-testid="rail">
        <button data-card onClick={activate}>
          A film
        </button>
      </div>
    </Screen>
  );
  const home = screen.getByRole("region", { name: "Home" });
  const rail = screen.getByTestId("rail");
  // Native scroll events do not bubble; a capture listener must see this.
  fireEvent(rail, new Event("scroll", { bubbles: false }));
  expect(home).toHaveClass("nova-scrolling");
  fireEvent.click(screen.getByRole("button", { name: "A film" }));
  expect(activate).toHaveBeenCalledOnce();
  act(() => vi.advanceTimersByTime(100));
  fireEvent(rail, new Event("scroll", { bubbles: false }));
  act(() => vi.advanceTimersByTime(100));
  expect(home).toHaveClass("nova-scrolling");
  act(() => vi.advanceTimersByTime(50));
  expect(home).not.toHaveClass("nova-scrolling");
});

it("does not start scroll work on inactive screens and cleans up pending work", () => {
  const view = render(
    <Screen active={false} label="Home" onScrolledChange={() => {}}>
      <div data-testid="rail" />
    </Screen>
  );
  const home = screen.getByRole("region", { hidden: true });
  fireEvent.scroll(screen.getByTestId("rail"));
  expect(home).not.toHaveClass("nova-scrolling");
  view.rerender(
    <Screen active label="Home" onScrolledChange={() => {}}>
      <div data-testid="rail" />
    </Screen>
  );
  const unrelatedTimers = vi.getTimerCount();
  fireEvent.scroll(screen.getByTestId("rail"));
  expect(vi.getTimerCount()).toBe(unrelatedTimers + 1);
  view.unmount();
  expect(vi.getTimerCount()).toBe(unrelatedTimers);
});
