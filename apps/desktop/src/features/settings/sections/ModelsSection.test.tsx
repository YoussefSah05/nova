import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { ModelsSection } from "./ModelsSection";
import type { ModelDownloadEvent } from "@/types/generated/ModelDownloadEvent";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn(), off: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
let installed = false;
function client() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
}
function mount(qc = client()) {
  return render(
    <QueryClientProvider client={qc}>
      <ModelsSection />
    </QueryClientProvider>
  );
}
beforeEach(() => {
  installed = false;
  vi.clearAllMocks();
  mocks.listen.mockResolvedValue(mocks.off);
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "get_model_status")
      return { installed, modelId: "whisper", availability: installed ? "available" : "missing" };
    if (command === "get_enrich_model_status")
      return { installed: true, modelId: "embedding", availability: "available" };
    if (command === "list_external_model_libraries" || command === "list_model_controls") return [];
    if (command === "model_runtime_status")
      return {
        activeJobs: 0,
        queuedJobs: 2,
        loadedModels: 0,
        estimatedResidentBytes: 0,
        unknownResidentModels: 1,
        suspendedWork: 2,
      };
  });
});
it("does not download on open and preserves a pending download across remounts", async () => {
  const user = userEvent.setup();
  const qc = client();
  let finish!: () => void;
  qc.setQueryData(["capability", "semanticSearch"], false);
  const base = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command: string) =>
    command === "download_whisper_model"
      ? new Promise<void>((resolve) => {
          finish = () => {
            installed = true;
            resolve();
          };
        })
      : base(command)
  );
  let progress!: (event: { payload: ModelDownloadEvent }) => void;
  mocks.listen.mockImplementation(async (_event, cb) => {
    progress = cb;
    return mocks.off;
  });
  const first = mount(qc);
  await screen.findByRole("button", { name: "Download captions (~500 MB)" });
  expect(mocks.invoke.mock.calls.some(([c]) => String(c).startsWith("download"))).toBe(false);
  await user.click(screen.getByRole("button", { name: "Download captions (~500 MB)" }));
  await waitFor(() => expect(progress).toBeDefined());
  act(() => progress({ payload: { artifact: "weights", downloadedBytes: 50, totalBytes: 100 } }));
  expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "50");
  first.unmount();
  mount(qc);
  expect(await screen.findByRole("button", { name: "Downloading…" })).toBeDisabled();
  await act(async () => finish());
  expect(await screen.findByRole("button", { name: "Remove model" })).toBeEnabled();
  expect(qc.getQueryState(["capability", "semanticSearch"])?.isInvalidated).toBe(true);
  expect(mocks.invoke.mock.calls.filter(([c]) => c === "download_whisper_model")).toHaveLength(1);
});
it("offers retry for model status errors without offering an unsafe download", async () => {
  const user = userEvent.setup();
  const base = mocks.invoke.getMockImplementation()!;
  let fails = true;
  mocks.invoke.mockImplementation((command: string) =>
    command === "get_model_status" && fails
      ? Promise.reject(new Error("private locator"))
      : base(command)
  );
  mount();
  await screen.findByText("Couldn’t check the caption model.");
  expect(screen.queryByRole("button", { name: /Download captions/ })).not.toBeInTheDocument();
  fails = false;
  await user.click(screen.getByRole("button", { name: "Try again" }));
  await screen.findByRole("button", { name: /Download captions/ });
  expect(document.body.textContent).not.toContain("private locator");
});
it("reports download errors and confirms removal", async () => {
  const user = userEvent.setup();
  const base = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command: string) =>
    command === "download_whisper_model"
      ? Promise.reject(new Error("download failed"))
      : base(command)
  );
  const first = mount();
  await user.click(await screen.findByRole("button", { name: /Download captions/ }));
  expect(await screen.findByRole("alert")).toHaveTextContent("The model action could not finish");
  first.unmount();
  installed = true;
  vi.spyOn(window, "confirm").mockReturnValue(false);
  mount();
  await user.click(await screen.findByRole("button", { name: "Remove model" }));
  expect(mocks.invoke.mock.calls.some(([c]) => c === "remove_whisper_model")).toBe(false);
});

it("refreshes Search capability after catalog-model installation", async () => {
  const user = userEvent.setup();
  const qc = client();
  let ready = false;
  const base = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "get_enrich_model_status")
      return {
        installed: ready,
        modelId: "embedding",
        availability: ready ? "available" : "missing",
      };
    if (command === "download_enrich_models") {
      ready = true;
      return;
    }
    return base(command);
  });
  const read = vi.fn(async () => ready);
  const capability = {
    queryKey: ["capability", "semanticSearch"],
    queryFn: read,
    staleTime: 300000,
  };
  expect(await qc.fetchQuery(capability)).toBe(false);
  mount(qc);
  await user.click(await screen.findByRole("button", { name: "Download catalog (~120 MB)" }));
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "Downloading…" })).not.toBeInTheDocument()
  );
  await waitFor(() => expect(qc.getQueryState(capability.queryKey)?.isInvalidated).toBe(true));
  expect(await qc.fetchQuery(capability)).toBe(true);
  expect(read).toHaveBeenCalledTimes(2);
});

it("downloads the missing translation directions with one shared progress bar", async () => {
  const user = userEvent.setup();
  let done = false;
  let finish!: () => void;
  const base = mocks.invoke.getMockImplementation()!;
  const translation = (modelId: string, ready: boolean) => ({
    modelId,
    capability: "translation",
    availability: ready ? "available" : "missing",
    library: "internal",
    enabled: true,
    activity: "idle",
  });
  mocks.invoke.mockImplementation((command: string) => {
    if (command === "list_model_controls")
      return Promise.resolve([
        translation("opus-mt-en-fr", true),
        translation("opus-mt-fr-en", done),
        translation("opus-mt-en-ar", done),
        translation("opus-mt-ar-en", done),
      ]);
    if (command === "download_translation_models")
      return new Promise<void>((resolve) => {
        finish = () => {
          done = true;
          resolve();
        };
      });
    return base(command);
  });
  let progress!: (event: { payload: ModelDownloadEvent }) => void;
  mocks.listen.mockImplementation(async (_event, cb) => {
    progress = cb;
    return mocks.off;
  });
  mount();
  expect(await screen.findByText(/1 of 4 directions installed/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Download the rest" }));
  await waitFor(() => expect(progress).toBeDefined());
  act(() =>
    progress({ payload: { artifact: "opus-mt-fr-en", downloadedBytes: 25, totalBytes: 100 } })
  );
  expect(screen.getByRole("progressbar", { name: "Translation model download" })).toHaveAttribute(
    "aria-valuenow",
    "25"
  );
  await act(async () => finish());
  expect(await screen.findByText(/4 of 4 directions installed/)).toBeInTheDocument();
  expect(mocks.invoke.mock.calls.filter(([c]) => c === "download_translation_models")).toHaveLength(
    1
  );
});
