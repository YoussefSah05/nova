import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useCaptionStartup } from "./useCaptionStartup";
const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  invoke.mockReset();
});

it("does not release captions on native playing until resume is confirmed", async () => {
  vi.useFakeTimers();
  invoke.mockResolvedValueOnce(null).mockResolvedValueOnce(125);
  const seed = vi.fn();
  const { result } = renderHook(() => useCaptionStartup("native", true, seed));
  await act(async () => {});
  expect(result.current).toBe(false);
  expect(seed).not.toHaveBeenCalled();
  await act(async () => vi.advanceTimersByTime(250));
  expect(result.current).toBe(true);
  expect(seed).toHaveBeenCalledWith(125);
});

it("ignores an old session's delayed confirmation", async () => {
  let resolve!: (position: number) => void;
  invoke
    .mockImplementationOnce(
      () =>
        new Promise<number>((done) => {
          resolve = done;
        })
    )
    .mockResolvedValue(null);
  const seed = vi.fn();
  const { result, rerender } = renderHook(({ token }) => useCaptionStartup(token, true, seed), {
    initialProps: { token: "old" },
  });
  rerender({ token: "new" });
  await act(async () => resolve(600));
  expect(result.current).toBe(false);
  expect(seed).not.toHaveBeenCalled();
});
