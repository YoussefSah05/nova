//! What every section shows before the first playlist: one sentence on what
//! Nova does with it, and one action.

import { Button } from "@/components/ui/button";
import { NovaMark } from "@/components/NovaLogo";
import { useSessionStatus } from "@/app/session";
import { useShell } from "@/app/shell";

export function NoPlaylist() {
  const { error } = useSessionStatus();
  const { openAddSource } = useShell();
  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-5 px-6 text-center">
      <NovaMark size={88} />
      <div className="flex flex-col gap-2">
        <h1 className="font-display text-[28px] font-medium">
          Your library starts with a playlist
        </h1>
        <p className="max-w-md text-sm leading-relaxed text-white/60">
          Add an M3U or Xtream playlist. Nova matches its films and shows, finds their artwork and
          builds your guide — all on this Mac.
        </p>
      </div>
      {error && <p className="max-w-md text-sm text-destructive">{error}</p>}
      <Button size="lg" onClick={openAddSource}>
        Add a playlist
      </Button>
    </div>
  );
}
