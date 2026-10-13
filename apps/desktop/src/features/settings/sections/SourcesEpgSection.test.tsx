import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { SourcesEpgSection } from "./SourcesEpgSection";
import { ConfirmProvider } from "../ConfirmProvider";
const mock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke: mock }));
vi.mock("./GuideStatusRow", () => ({ GuideStatusRow: () => null }));
beforeEach(() => vi.clearAllMocks());
function mount(ui: React.ReactElement) {
  return render(<ConfirmProvider>{ui}</ConfirmProvider>);
}
const source = {
  id: 1,
  name: "Fixture",
  kind: "m3u",
  displayUrl: "example.invalid",
  autoRefreshHours: 24,
};
it("distinguishes a load failure from an empty library and offers setup after retry", async () => {
  const user = userEvent.setup();
  const add = vi.fn();
  mock.mockRejectedValueOnce(new Error("read failed")).mockResolvedValue([]);
  mount(<SourcesEpgSection profileId="p" onSourcesChanged={() => {}} onAddSource={add} />);
  await screen.findByRole("alert");
  expect(screen.queryByText("No playlists yet")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Try again" }));
  await user.click(await screen.findByRole("button", { name: "Add a playlist" }));
  expect(add).toHaveBeenCalled();
});
it("keeps the playlist visible after refresh failure and does not expose raw errors", async () => {
  const user = userEvent.setup();
  mock.mockImplementation(async (command: string) => {
    if (command === "list_sources") return [source];
    throw new Error("https://private-user:secret@example.invalid");
  });
  mount(<SourcesEpgSection profileId="p" onSourcesChanged={() => {}} onAddSource={() => {}} />);
  await user.click(await screen.findByRole("button", { name: "Refresh" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Your existing catalog is still available"
  );
  expect(screen.getByText("Fixture · M3U")).toBeInTheDocument();
  expect(document.body.textContent).not.toContain("secret");
});
it("removes a playlist only after the in-app confirmation", async () => {
  const user = userEvent.setup();
  const changed = vi.fn();
  let sources = [source];
  mock.mockImplementation(async (command: string) => {
    if (command === "list_sources") return sources;
    if (command === "delete_source") sources = [];
  });
  mount(<SourcesEpgSection profileId="p" onSourcesChanged={changed} onAddSource={() => {}} />);
  await user.click(await screen.findByRole("button", { name: "Remove" }));
  const dialog = await screen.findByRole("alertdialog", { name: "Remove “Fixture”?" });
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(mock).not.toHaveBeenCalledWith("delete_source", expect.anything());

  await user.click(screen.getByRole("button", { name: "Remove" }));
  await user.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Remove playlist" })
  );
  await waitFor(() => expect(changed).toHaveBeenCalled());
  expect(mock).toHaveBeenCalledWith("delete_source", { profileId: "p", sourceId: 1 });
  expect(await screen.findByText("No playlists yet")).toBeInTheDocument();
});
