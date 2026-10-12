import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { FrameRateOverlay } from "./FrameRateOverlay";

afterEach(() => window.localStorage.clear());

it("stays off until toggled and remembers the choice", () => {
  const first = render(<FrameRateOverlay />);
  expect(screen.queryByRole("status", { name: "Frame rate" })).not.toBeInTheDocument();
  act(() => {
    fireEvent.keyDown(window, { code: "KeyF", key: "ƒ", metaKey: true, altKey: true });
  });
  expect(screen.getByRole("status", { name: "Frame rate" })).toBeInTheDocument();
  first.unmount();
  render(<FrameRateOverlay />);
  expect(screen.getByRole("status", { name: "Frame rate" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Hide frame rate" }));
  expect(screen.queryByRole("status", { name: "Frame rate" })).not.toBeInTheDocument();
  expect(window.localStorage.getItem("nova.frameRateOverlay")).toBeNull();
});
