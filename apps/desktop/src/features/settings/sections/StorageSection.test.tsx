import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import { StorageSection } from "./StorageSection";
import { ConfirmProvider } from "../ConfirmProvider";
const mock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock }));
function mount(client: QueryClient, profileId = "p") {
  return render(
    <QueryClientProvider client={client}>
      <ConfirmProvider>
        <StorageSection profileId={profileId} />
      </ConfirmProvider>
    </QueryClientProvider>
  );
}
beforeEach(() => vi.clearAllMocks());
it("refreshes cached counts on reopen and scopes clearing and Moments invalidation", async () => {
  const user = userEvent.setup();
  let count = 0;
  mock.mockImplementation(async (command: string) => {
    if (command === "get_transcription_cache_stats") return { transcripts: count, bytes: 100 };
    if (command === "get_artwork_cache_stats") return { files: 2, bytes: 1024 };
    if (command === "clear_transcription_cache") count = 0;
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 300000 } },
  });
  const key = ["browse", "p", 1, "search", "moments", {}];
  const other = ["browse", "other", 1, "search", "moments", {}];
  client.setQueryData(key, []);
  client.setQueryData(other, []);
  const first = mount(client);
  expect(await screen.findByRole("button", { name: "Clear transcripts" })).toBeDisabled();
  first.unmount();
  count = 3;
  mount(client);
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Clear transcripts" })).toBeEnabled()
  );
  await user.click(screen.getByRole("button", { name: "Clear transcripts" }));
  await user.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "Clear transcripts",
    })
  );
  await screen.findByText("Cache cleared.");
  expect(mock).toHaveBeenCalledWith("clear_transcription_cache", { profileId: "p" });
  expect(client.getQueryState(key)?.isInvalidated).toBe(true);
  expect(client.getQueryState(other)?.isInvalidated).toBe(false);
  expect(screen.getByRole("button", { name: "Clear transcripts" })).toBeDisabled();
});
it("reports a failed cache clear without claiming success", async () => {
  const user = userEvent.setup();
  mock.mockImplementation(async (command: string) => {
    if (command === "get_transcription_cache_stats") return { transcripts: 1, bytes: 100 };
    if (command === "get_artwork_cache_stats") return { files: 2, bytes: 1024 };
    throw new Error("private path");
  });
  mount(new QueryClient({ defaultOptions: { queries: { retry: false } } }));
  await user.click(await screen.findByRole("button", { name: "Clear artwork" }));
  await user.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Clear artwork" })
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("Couldn’t clear cached artwork");
  expect(screen.queryByText("Cache cleared.")).not.toBeInTheDocument();
  expect(document.body.textContent).not.toContain("private path");
});
