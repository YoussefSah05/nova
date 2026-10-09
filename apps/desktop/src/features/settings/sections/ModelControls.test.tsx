import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { ModelControls } from "./ModelControls";
const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
function mount() {
  render(
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <ModelControls />
    </QueryClientProvider>
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  mocks.invoke.mockImplementation(async (command: string) =>
    command === "list_model_controls"
      ? [
          {
            modelId: "opus-mt-en-fr",
            capability: "translation",
            availability: "available",
            library: "external",
            enabled: true,
            activity: "loaded",
          },
          {
            modelId: "opus-mt-fr-en",
            capability: "translation",
            availability: "missing",
            library: "internal",
            enabled: false,
            activity: "idle",
          },
          {
            modelId: "large-v3-turbo-q5_0",
            capability: "transcription",
            availability: "available",
            library: "internal",
            enabled: true,
            activity: "inUse",
          },
        ]
      : undefined
  );
});
it("shows effective status without loading and toggles the exact model", async () => {
  const user = userEvent.setup();
  mount();
  await screen.findByText("English → French");
  expect(
    screen.getByText("Installed · External library · Enabled · Loaded in memory")
  ).toBeInTheDocument();
  expect(mocks.invoke.mock.calls.every(([command]) => command === "list_model_controls")).toBe(
    true
  );
  expect(screen.getByRole("button", { name: "Disable Whisper · source captions" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Disable English → French" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("set_model_enabled", {
      modelId: "opus-mt-en-fr",
      enabled: false,
    })
  );
  await user.click(screen.getByRole("button", { name: "Enable French → English" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("set_model_enabled", {
      modelId: "opus-mt-fr-en",
      enabled: true,
    })
  );
});
it("unloads without disabling and presents safe actionable failures", async () => {
  const user = userEvent.setup();
  const base = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "unload_model" ? Promise.reject(new Error("private path")) : base(command, args)
  );
  mount();
  await user.click(await screen.findByRole("button", { name: "Free memory for English → French" }));
  expect(mocks.invoke).toHaveBeenCalledWith("unload_model", { modelId: "opus-mt-en-fr" });
  expect(await screen.findByRole("alert")).toHaveTextContent("Turn captions off");
  expect(within(document.body).queryByText("private path")).not.toBeInTheDocument();
  expect(mocks.invoke.mock.calls.some(([command]) => command === "set_model_enabled")).toBe(false);
});
