import { describe, expect, it } from "vitest";
import { formatRelative } from "./formatRelative";

const NOW_MS = 1_700_000_000_000; // fixed "now"
const NOW_S = Math.floor(NOW_MS / 1000);

describe("formatRelative", () => {
  it("treats sub-minute and future timestamps as 'just now'", () => {
    expect(formatRelative(NOW_S, NOW_MS)).toBe("just now");
    expect(formatRelative(NOW_S - 30, NOW_MS)).toBe("just now");
    expect(formatRelative(NOW_S + 500, NOW_MS)).toBe("just now");
  });

  it("formats minutes and hours", () => {
    expect(formatRelative(NOW_S - 5 * 60, NOW_MS)).toBe("5m ago");
    expect(formatRelative(NOW_S - 3 * 3600, NOW_MS)).toBe("3h ago");
  });

  it("formats days and weeks", () => {
    expect(formatRelative(NOW_S - 24 * 3600, NOW_MS)).toBe("yesterday");
    expect(formatRelative(NOW_S - 3 * 24 * 3600, NOW_MS)).toBe("3 days ago");
    expect(formatRelative(NOW_S - 14 * 24 * 3600, NOW_MS)).toBe("2w ago");
  });

  it("falls back to an absolute date past ~a month", () => {
    const out = formatRelative(NOW_S - 60 * 24 * 3600, NOW_MS);
    expect(out).not.toMatch(/ago|yesterday|just now/);
  });
});
