import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import { DEFAULT_PROFILE_SETTINGS, type ProfileSettings } from "@/types";
import { EnrichmentSection } from "./EnrichmentSection";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
const mockInvoke = vi.mocked(invoke);

const TOKEN = "eyJhbGciOiJIUzI1NiJ9.eyJhdWQiOiJzZWNyZXQifQ.c2lnbmF0dXJlWXYZ";

/// A fake backend with the real contract: the credential is write-only.
function backend(initial: Partial<ProfileSettings> = {}) {
  let stored: string | null = null;
  let settings: ProfileSettings = { ...DEFAULT_PROFILE_SETTINGS, ...initial };
  mockInvoke.mockImplementation(async (command, args) => {
    const a = args as Record<string, unknown> | undefined;
    switch (command) {
      case "get_enrich_model_status":
        return { installed: true, modelId: "e5" };
      case "get_profile_settings":
        return {
          ...settings,
          tmdbCredentialConfigured: stored !== null,
          tmdbCredentialHint: stored ? stored.slice(-4) : null,
        };
      case "set_profile_settings":
        settings = a?.settings as ProfileSettings;
        return undefined;
      case "set_tmdb_credential":
        stored = (a?.credential as string).trim();
        return undefined;
      case "clear_tmdb_credential":
        stored = null;
        return undefined;
      default:
        return undefined;
    }
  });
  return { stored: () => stored };
}

async function renderSection() {
  render(<EnrichmentSection profileId="p1" />);
  await act(async () => {});
}

describe("TMDB credential in settings", () => {
  beforeEach(() => mockInvoke.mockReset());

  it("is entered masked, saved write-only, and never shown again", async () => {
    const api = backend();
    await renderSection();
    const input = screen.getByLabelText("TMDB API Read Access Token") as HTMLInputElement;
    expect(input.type).toBe("password");
    expect(input.autocomplete).toBe("off");

    fireEvent.change(input, { target: { value: TOKEN } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
    });

    expect(api.stored()).toBe(TOKEN);
    expect(mockInvoke).toHaveBeenCalledWith("set_tmdb_credential", {
      profileId: "p1",
      credential: TOKEN,
    });
    // Only the hint is rendered; the token is nowhere in the page.
    expect(screen.getByLabelText("Saved TMDB token").textContent).toContain("••••WXYZ");
    expect(document.body.innerHTML).not.toContain(TOKEN.slice(0, 20));
    expect(screen.queryByLabelText("TMDB API Read Access Token")).toBeNull();
  });

  it("never sends the credential with other settings", async () => {
    backend();
    await renderSection();
    await act(async () => {
      fireEvent.click(screen.getByRole("switch", { name: "Use TMDB" }));
    });
    const call = mockInvoke.mock.calls.find(([c]) => c === "set_profile_settings");
    expect(call).toBeDefined();
    expect(JSON.stringify(call?.[1])).not.toContain("tmdbApiKey");
  });

  it("can replace or remove a saved token without revealing it", async () => {
    const api = backend();
    await renderSection();
    fireEvent.change(screen.getByLabelText("TMDB API Read Access Token"), {
      target: { value: TOKEN },
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
    });

    fireEvent.click(screen.getByRole("button", { name: "Replace" }));
    const input = screen.getByLabelText("TMDB API Read Access Token") as HTMLInputElement;
    // The replace field starts empty: the saved token is never pre-filled.
    expect(input.value).toBe("");

    vi.spyOn(window, "confirm").mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    });
    expect(api.stored()).toBeNull();
    expect(screen.getByLabelText("TMDB API Read Access Token")).toBeTruthy();
  });
});
