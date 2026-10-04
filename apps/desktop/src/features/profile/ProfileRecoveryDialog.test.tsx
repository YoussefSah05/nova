import { useState } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Profile } from "@/types";
import { ProfileRecoveryDialog } from "./ProfileRecoveryDialog";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
const original: Profile = {
  id: "original",
  name: "Alex",
  avatarColor: "#e0a33e",
  createdAt: 1,
  lastUsedAt: null,
};
const recovered: Profile = { ...original, id: "recovered", name: "Alex (Recovered)" };

beforeEach(() => {
  invoke.mockReset();
});

describe("profile copy recovery", () => {
  it("checks support before showing the recovery action and keeps unsupported data untouched", async () => {
    invoke.mockResolvedValue("unsupported");
    render(
      <ProfileRecoveryDialog
        profile={original}
        onRecovered={vi.fn()}
        onOpen={vi.fn()}
        onClose={vi.fn()}
      />
    );
    expect(await screen.findByText(/can’t safely recover/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Recover a copy" })).not.toBeInTheDocument();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("inspect_profile_recovery", {
      profileId: "original",
    });
  });

  it("guards repeated recovery, keeps the dialog open while pending, and opens only on explicit selection", async () => {
    let finish: ((profile: Profile) => void) | undefined;
    invoke.mockImplementation((command: string) =>
      command === "inspect_profile_recovery"
        ? Promise.resolve("available")
        : new Promise<Profile>((resolve) => {
            finish = resolve;
          })
    );
    const onRecovered = vi.fn();
    const onOpen = vi.fn();
    const onClose = vi.fn();
    render(
      <ProfileRecoveryDialog
        profile={original}
        onRecovered={onRecovered}
        onOpen={onOpen}
        onClose={onClose}
      />
    );
    const recover = await screen.findByRole("button", { name: "Recover a copy" });
    fireEvent.click(recover);
    fireEvent.click(recover);
    expect(
      invoke.mock.calls.filter(([command]) => command === "recover_profile_copy")
    ).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Close" })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    finish?.(recovered);
    const open = await screen.findByRole("button", { name: "Open recovered profile" });
    expect(onRecovered).toHaveBeenCalledWith(recovered);
    expect(onOpen).not.toHaveBeenCalled();
    await userEvent.click(open);
    expect(onOpen).toHaveBeenCalledWith(recovered);
  });

  it("handles a failed copy without inventing a recovered profile and allows retry", async () => {
    invoke
      .mockResolvedValueOnce("available")
      .mockRejectedValueOnce({ kind: "Conflict", message: "private diagnostic" })
      .mockResolvedValueOnce(recovered);
    const onRecovered = vi.fn();
    render(
      <ProfileRecoveryDialog
        profile={original}
        onRecovered={onRecovered}
        onOpen={vi.fn()}
        onClose={vi.fn()}
      />
    );
    await userEvent.click(await screen.findByRole("button", { name: "Recover a copy" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("original profile is unchanged");
    expect(screen.queryByText("private diagnostic")).not.toBeInTheDocument();
    expect(onRecovered).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Recover a copy" }));
    expect(
      await screen.findByRole("button", { name: "Open recovered profile" })
    ).toBeInTheDocument();
    expect(onRecovered).toHaveBeenCalledTimes(1);
  });

  it("traps focus and returns it to the opening control on Escape", async () => {
    invoke.mockResolvedValue("available");
    function Fixture() {
      const [show, setShow] = useState(false);
      return (
        <>
          <button onClick={() => setShow(true)}>Open fixture</button>
          {show && (
            <ProfileRecoveryDialog
              profile={original}
              onRecovered={vi.fn()}
              onOpen={vi.fn()}
              onClose={() => setShow(false)}
            />
          )}
        </>
      );
    }
    render(<Fixture />);
    const trigger = screen.getByRole("button", { name: "Open fixture" });
    await userEvent.click(trigger);
    await screen.findByRole("button", { name: "Recover a copy" });
    await userEvent.tab();
    await userEvent.tab();
    await userEvent.tab();
    expect(screen.getByRole("dialog")).toContainElement(document.activeElement as HTMLElement);
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => expect(trigger).toHaveFocus());
  });
});
