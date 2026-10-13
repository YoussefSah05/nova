import { useCallback, useRef, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { cn } from "@/lib/utils";
import { ConfirmContext, type ConfirmRequest } from "./confirm";

interface Pending extends ConfirmRequest {
  resolve: (answer: boolean) => void;
}

/** Hosts the confirmation dialog that `useConfirm()` opens. One question at a
 * time: asking again while a question is open answers the old one "no". */
export function ConfirmProvider({ children }: { children: React.ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  const confirm = useCallback(
    (request: ConfirmRequest) =>
      new Promise<boolean>((resolve) => {
        setPending((previous) => {
          previous?.resolve(false);
          return { ...request, resolve };
        });
      }),
    []
  );

  const answer = (value: boolean) => {
    pending?.resolve(value);
    setPending(null);
  };

  return (
    <ConfirmContext.Provider value={confirm}>
      {children}
      <Dialog.Root
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open) answer(false);
        }}
      >
        <Dialog.Portal>
          <Dialog.Overlay className="fixed inset-0 z-[60] bg-background/60 backdrop-blur-sm" />
          <Dialog.Content
            role="alertdialog"
            onOpenAutoFocus={(event) => {
              // The safe choice has focus, so Enter never destroys anything.
              event.preventDefault();
              cancelRef.current?.focus();
            }}
            className="nova-glass fixed left-1/2 top-1/2 z-[60] flex w-[calc(100%-2rem)] max-w-md -translate-x-1/2 -translate-y-1/2 flex-col gap-3 rounded-2xl p-6 outline-none"
          >
            <Dialog.Title className="font-display text-[20px] font-medium">
              {pending?.title}
            </Dialog.Title>
            <Dialog.Description className="text-sm leading-relaxed text-muted-foreground">
              {pending?.description}
            </Dialog.Description>
            <div className="mt-3 flex justify-end gap-2">
              <button
                ref={cancelRef}
                type="button"
                onClick={() => answer(false)}
                className="rounded-lg bg-white/10 px-4 py-2 text-sm font-medium text-white/85 transition-colors hover:bg-white/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => answer(true)}
                className={cn(
                  "rounded-lg px-4 py-2 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  pending?.destructive
                    ? "bg-rose-500/80 text-white hover:bg-rose-500"
                    : "bg-primary text-primary-foreground hover:brightness-110"
                )}
              >
                {pending?.confirmLabel}
              </button>
            </div>
          </Dialog.Content>
        </Dialog.Portal>
      </Dialog.Root>
    </ConfirmContext.Provider>
  );
}
