import { useEffect, useRef, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { invoke } from "@tauri-apps/api/core";
import { ShieldCheck } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { Profile } from "@/types";
import type { ProfileRecoveryStatus } from "@/types/generated/ProfileRecoveryStatus";

interface Props {
  profile: Profile;
  onRecovered: (profile: Profile) => void;
  onOpen: (profile: Profile) => void;
  onClose: () => void;
}

export function ProfileRecoveryDialog({ profile, onRecovered, onOpen, onClose }: Props) {
  const [status, setStatus] = useState<ProfileRecoveryStatus | "checking">("checking");
  const [pending, setPending] = useState(false);
  const [recovered, setRecovered] = useState<Profile | null>(null);
  const [error, setError] = useState<string | null>(null);
  const submitting = useRef(false);
  const returnFocus = useRef(
    document.activeElement instanceof HTMLElement ? document.activeElement : null
  );

  useEffect(() => {
    let live = true;
    void invoke<ProfileRecoveryStatus>("inspect_profile_recovery", { profileId: profile.id })
      .then((result) => live && setStatus(result))
      .catch(() => live && setStatus("unsupported"));
    return () => {
      live = false;
    };
  }, [profile.id]);

  async function recover() {
    if (submitting.current) return;
    submitting.current = true;
    setPending(true);
    setError(null);
    try {
      const copy = await invoke<Profile>("recover_profile_copy", { profileId: profile.id });
      setRecovered(copy);
      onRecovered(copy);
    } catch {
      setError(
        "Recovery couldn’t be completed. Your original profile is unchanged. You can try again."
      );
    } finally {
      submitting.current = false;
      setPending(false);
    }
  }

  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open && !pending) onClose();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm" />
        <Dialog.Content
          className="nova-glass fixed left-1/2 top-1/2 z-50 flex w-[calc(100%-2rem)] max-w-md -translate-x-1/2 -translate-y-1/2 flex-col gap-4 rounded-2xl p-6"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            returnFocus.current?.focus();
          }}
          onEscapeKeyDown={(event) => {
            if (pending) event.preventDefault();
          }}
          onInteractOutside={(event) => {
            if (pending) event.preventDefault();
          }}
        >
          <ShieldCheck className="h-7 w-7 text-primary" aria-hidden />
          <Dialog.Title className="font-display text-[22px] font-medium">
            {recovered ? "Your recovered profile is ready" : `Recover ${profile.name}`}
          </Dialog.Title>
          <Dialog.Description className="text-sm leading-relaxed text-muted-foreground">
            {recovered
              ? "Your playlists, viewing history and saved captions are ready in a separate profile. The original profile is still available as your backup."
              : "This profile couldn’t be opened. Nova can check whether its data can be recovered into a separate profile, keeping the original as your backup."}
          </Dialog.Description>
          <div role="status" aria-live="polite" className="text-sm text-muted-foreground">
            {status === "checking" && "Checking recovery options…"}
            {status === "unsupported" &&
              "Nova can’t safely recover this profile yet. Its original data has been kept."}
            {status === "available" &&
              !pending &&
              !recovered &&
              "A compatible recovery path is available. The copy is added only after all checks pass."}
            {pending &&
              "Creating and checking your copy… This may take a little while. Please keep Nova open."}
          </div>
          {error && (
            <p role="alert" className="text-sm text-rose-300">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-3">
            <Button variant="secondary" disabled={pending} onClick={onClose}>
              {recovered ? "Back to profiles" : "Close"}
            </Button>
            {status === "available" && !recovered && (
              <Button onClick={() => void recover()} disabled={pending}>
                {pending ? "Recovering…" : "Recover a copy"}
              </Button>
            )}
            {recovered && <Button onClick={() => onOpen(recovered)}>Open recovered profile</Button>}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
