import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { VersionsPanel } from "./VersionsPanel";

afterEach(cleanup);
it("selects a source before playback and marks the transient selection", async () => {
  const select = vi.fn();
  const variants = ["a", "b"].map((id) => ({
    variantId: id,
    sourceId: 1,
    sourceName: `Source ${id}`,
    label: id,
    available: true,
    badges: [],
    recentlyFailed: false,
    picked: id === "a",
  }));
  const { rerender } = render(
    <VersionsPanel
      variants={variants}
      preferredLang={null}
      selectedVariantId="a"
      onSelect={select}
    />
  );
  await userEvent.click(screen.getByRole("button", { name: "Select Source b" }));
  expect(select).toHaveBeenCalledWith("b");
  rerender(
    <VersionsPanel
      variants={variants}
      preferredLang={null}
      selectedVariantId="b"
      onSelect={select}
    />
  );
  expect(screen.getByRole("button", { name: "Select Source b" })).toHaveAttribute(
    "aria-pressed",
    "true"
  );
  expect(screen.getByRole("button", { name: /Select Source a/ })).toHaveAttribute(
    "aria-pressed",
    "false"
  );
});
