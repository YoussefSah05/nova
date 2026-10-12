import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NEUTRAL_TONE, cachedLogoTone, resetLogoTones, sampleLogoTone } from "./logoTone";
import { isScrolling, whenScrollIdle } from "./scrollActivity";

const KEY = "nova.logoTones.v1";
const RED = { mode: "tint", background: "linear-gradient(red, black)" };

beforeEach(() => {
  window.localStorage.clear();
  resetLogoTones();
});
afterEach(() => vi.useRealTimers());

it("reuses tones stored by an earlier launch and ignores corrupt entries", () => {
  window.localStorage.setItem(
    KEY,
    JSON.stringify({ "https://x/a.png": RED, "https://x/b.png": { mode: "nope", background: 1 } })
  );
  expect(cachedLogoTone("https://x/a.png")).toEqual(RED);
  expect(cachedLogoTone("https://x/b.png")).toBeUndefined();
});

it("does not persist a logo that could not be analysed", async () => {
  vi.useFakeTimers();
  window.localStorage.setItem(KEY, JSON.stringify({ "https://x/a.png": RED }));
  // jsdom cannot decode images: the analysis falls back to the neutral tone.
  const tone = sampleLogoTone("https://x/broken.png");
  await vi.runAllTimersAsync();
  expect(await tone).toEqual(NEUTRAL_TONE);
  await vi.runAllTimersAsync();
  const stored = JSON.parse(window.localStorage.getItem(KEY) ?? "{}");
  expect(stored).toEqual({ "https://x/a.png": RED });
});

it("waits for scrolling to stop", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
  expect(isScrolling()).toBe(false); // installs the listener
  document.dispatchEvent(new Event("scroll"));
  expect(isScrolling()).toBe(true);
  let idle = false;
  void whenScrollIdle().then(() => (idle = true));
  await vi.advanceTimersByTimeAsync(100);
  expect(idle).toBe(false);
  await vi.advanceTimersByTimeAsync(300);
  expect(idle).toBe(true);
});
