import { useState } from "react";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { SimpleSelect } from "@/components/ui/select";
import { SettingsModal } from "./SettingsModal";

vi.mock("./sections/PlaybackCaptionsSection", () => ({
  PlaybackCaptionsSection: () => (
    <SimpleSelect
      value="auto"
      onChange={() => {}}
      ariaLabel="Route"
      options={[
        { value: "auto", label: "Automatic" },
        { value: "manual", label: "Manual" },
      ]}
    />
  ),
}));
vi.mock("./sections/AppearanceSection", () => ({
  AppearanceSection: () => <p>Appearance controls</p>,
}));
vi.mock("./sections/ModelsSection", () => ({ ModelsSection: () => <p>Model controls</p> }));
vi.mock("./sections/StorageSection", () => ({ StorageSection: () => <p>Storage controls</p> }));
vi.mock("./sections/EnrichmentSection", () => ({
  EnrichmentSection: () => <p>Catalog controls</p>,
}));
vi.mock("./sections/SourcesEpgSection", () => ({
  SourcesEpgSection: ({ onAddSource }: { onAddSource: () => void }) => (
    <button onClick={onAddSource}>Add a playlist</button>
  ),
}));
const deleted = vi.hoisted(() => vi.fn());
vi.mock("./sections/ProfileAboutSection", async () => {
  const { useConfirm } = await import("./confirm");
  return {
    ProfileAboutSection: () => {
      const confirm = useConfirm();
      return (
        <button
          onClick={() =>
            void confirm({
              title: "Delete “Review”?",
              description: "Its library is deleted.",
              confirmLabel: "Delete profile",
              destructive: true,
            }).then((yes) => yes && deleted())
          }
        >
          Delete profile
        </button>
      );
    },
  };
});
const profile = { id: "p", name: "Review", avatarColor: "#ffffff", createdAt: 0, lastUsedAt: null };
function Host() {
  const [open, setOpen] = useState(false);
  const [adding, setAdding] = useState(false);
  return (
    <>
      <button onClick={() => setOpen(true)}>Open settings</button>
      {adding && <input aria-label="Playlist URL" autoFocus />}
      {open && (
        <SettingsModal
          profile={profile}
          onClose={() => setOpen(false)}
          onSwitchProfile={() => {}}
          onProfileUpdated={() => {}}
          onSourcesChanged={() => {}}
          onAddSource={() => {
            setOpen(false);
            setAdding(true);
          }}
        />
      )}
    </>
  );
}
it("contains keyboard focus, switches sections and returns focus after Escape", async () => {
  HTMLElement.prototype.scrollTo = vi.fn();
  const user = userEvent.setup();
  render(<Host />);
  await user.click(screen.getByRole("button", { name: "Open settings" }));
  const dialog = screen.getByRole("dialog", { name: "Settings" });
  expect(dialog).toHaveAttribute("aria-modal", "true");
  expect(dialog).toContainElement(document.activeElement as HTMLElement);
  const close = within(dialog).getByRole("button", { name: "Close settings" });
  for (let i = 0; i < 12; i++) {
    await user.tab();
    expect(dialog).toContainElement(document.activeElement as HTMLElement);
  }
  await user.click(screen.getByRole("tab", { name: "Models" }));
  expect(screen.getByText("Model controls")).toBeInTheDocument();
  await user.keyboard("{ArrowDown}");
  expect(screen.getByRole("tab", { name: "Storage" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByText("Storage controls")).toBeInTheDocument();
  act(() => close.focus());
  await user.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(screen.getByRole("button", { name: "Open settings" })).toHaveFocus();
});
it("keeps portalled selects usable inside the modal", async () => {
  Element.prototype.scrollIntoView = vi.fn();
  const user = userEvent.setup();
  render(<Host />);
  await user.click(screen.getByRole("button", { name: "Open settings" }));
  act(() => screen.getByRole("combobox", { name: "Route" }).focus());
  await user.keyboard("{ArrowDown}");
  expect(await screen.findByRole("option", { name: "Manual" })).toBeInTheDocument();
  await user.keyboard("{Escape}");
  expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Close settings" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});

it("keeps focus in the next form when Settings hands off to Add playlist", async () => {
  HTMLElement.prototype.scrollTo = vi.fn();
  const user = userEvent.setup();
  render(<Host />);
  await user.click(screen.getByRole("button", { name: "Open settings" }));
  await user.click(screen.getByRole("tab", { name: "Playlists & guide" }));
  await user.click(screen.getByRole("button", { name: "Add a playlist" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(screen.getByRole("textbox", { name: "Playlist URL" })).toHaveFocus();
});

it("confirms destructive actions in a dialog layered over Settings", async () => {
  const user = userEvent.setup();
  render(<Host />);
  await user.click(screen.getByRole("button", { name: "Open settings" }));
  await user.click(await screen.findByRole("tab", { name: "Profile & about" }));
  await user.click(await screen.findByRole("button", { name: "Delete profile" }));
  const confirm = await screen.findByRole("alertdialog", { name: "Delete “Review”?" });
  // Escape answers the confirmation, not Settings.
  await user.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
  expect(deleted).not.toHaveBeenCalled();
  expect(confirm).not.toBeInTheDocument();

  await user.click(screen.getByRole("button", { name: "Delete profile" }));
  await user.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", { name: "Delete profile" })
  );
  await waitFor(() => expect(deleted).toHaveBeenCalledOnce());
  expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
});
