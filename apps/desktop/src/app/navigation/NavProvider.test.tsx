import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { NavProvider } from "./NavProvider";
import { useNav } from "./useNav";

afterEach(() => {
  cleanup();
  Reflect.deleteProperty(document, "startViewTransition");
  vi.unstubAllGlobals();
});

function NavigationControls() {
  const nav = useNav();
  return (
    <>
      <button onClick={() => nav.switchTo("home")}>Home</button>
      <button onClick={() => nav.push({ kind: "search", query: "film" })}>Search</button>
      <output>{nav.current.route.kind}</output>
    </>
  );
}

it("skips a transition for the active root but keeps reselect-to-root behavior", () => {
  const transition = vi.fn((update: () => void) => update());
  Object.defineProperty(document, "startViewTransition", { configurable: true, value: transition });
  render(
    <NavProvider blocked={() => false}>
      <NavigationControls />
    </NavProvider>
  );
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  expect(transition).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Search" }));
  expect(screen.getByRole("status")).toHaveTextContent("search");
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  expect(screen.getByRole("status")).toHaveTextContent("root");
  expect(transition).toHaveBeenCalledTimes(2);
});

it("navigates immediately without view snapshots when motion is reduced", () => {
  const transition = vi.fn();
  Object.defineProperty(document, "startViewTransition", { configurable: true, value: transition });
  vi.stubGlobal("matchMedia", () => ({ matches: true }));
  render(
    <NavProvider blocked={() => false}>
      <NavigationControls />
    </NavProvider>
  );
  fireEvent.click(screen.getByRole("button", { name: "Search" }));
  expect(screen.getByRole("status")).toHaveTextContent("search");
  expect(transition).not.toHaveBeenCalled();
});
