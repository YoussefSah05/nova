import { act, render, renderHook, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it } from "vitest";
import { ConfirmProvider } from "./ConfirmProvider";
import { useConfirm, type Confirm } from "./confirm";

const request = {
  title: "Remove “News”?",
  description: "Its channels are removed from this profile.",
  confirmLabel: "Remove playlist",
  destructive: true,
};

function Harness({ onReady }: { onReady: (confirm: Confirm) => void }) {
  onReady(useConfirm());
  return null;
}

function mount() {
  let confirm!: Confirm;
  render(
    <ConfirmProvider>
      <Harness onReady={(c) => (confirm = c)} />
    </ConfirmProvider>
  );
  return () => confirm;
}

it("asks in an accessible dialog and resolves with the answer", async () => {
  const user = userEvent.setup();
  const confirm = mount();
  let answer: Promise<boolean>;
  act(() => {
    answer = confirm()(request);
  });
  const dialog = await screen.findByRole("alertdialog", { name: "Remove “News”?" });
  expect(dialog).toHaveAccessibleDescription("Its channels are removed from this profile.");
  expect(within(dialog).getByRole("button", { name: "Cancel" })).toHaveFocus();
  await user.click(within(dialog).getByRole("button", { name: "Remove playlist" }));
  await expect(answer!).resolves.toBe(true);
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
});

it("treats Escape, Cancel and a newer question as no", async () => {
  const user = userEvent.setup();
  const confirm = mount();
  let first: Promise<boolean>;
  let second: Promise<boolean>;
  act(() => {
    first = confirm()(request);
  });
  await screen.findByRole("alertdialog");
  await user.keyboard("{Escape}");
  await expect(first!).resolves.toBe(false);

  act(() => {
    first = confirm()(request);
  });
  act(() => {
    second = confirm()({ ...request, title: "Clear artwork?" });
  });
  await expect(first!).resolves.toBe(false);
  const dialog = await screen.findByRole("alertdialog", { name: "Clear artwork?" });
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  await expect(second!).resolves.toBe(false);
});

it("fails loudly without a provider instead of silently refusing", () => {
  expect(() => renderHook(() => useConfirm())).toThrow(/ConfirmProvider/);
});
