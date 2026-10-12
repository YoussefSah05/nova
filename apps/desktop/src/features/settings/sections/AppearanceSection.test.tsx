import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { AppearanceSection } from "./AppearanceSection";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../useProfileSettings", () => ({
  useProfileSettings: () => ({ settings: { accentColor: "amber" }, setSettings: vi.fn() }),
}));

let stored = true;
beforeEach(() => {
  stored = true;
  mocks.invoke.mockReset();
  mocks.invoke.mockImplementation(async (command: string, args?: { enabled: boolean }) => {
    if (command === "get_high_refresh_rate") return stored;
    if (command === "set_high_refresh_rate") stored = args!.enabled;
  });
});

it("shows the stored refresh-rate choice and saves changes", async () => {
  const user = userEvent.setup();
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <AppearanceSection profileId="p" />
    </QueryClientProvider>
  );
  const toggle = await screen.findByRole("switch", { name: "High refresh rate" });
  await waitFor(() => expect(toggle).toBeChecked());
  await user.click(toggle);
  expect(mocks.invoke).toHaveBeenCalledWith("set_high_refresh_rate", { enabled: false });
  await waitFor(() => expect(toggle).not.toBeChecked());
});
