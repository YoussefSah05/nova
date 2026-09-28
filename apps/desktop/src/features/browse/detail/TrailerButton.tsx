//! Opens a title's trailer in the default browser (YouTube only; the backend
//! refuses anything else).

import { invoke } from "@tauri-apps/api/core";
import { Clapperboard } from "lucide-react";

import { Button } from "@/components/ui/button";

export function TrailerButton({ url }: { url: string | null | undefined }) {
  if (!url) return null;
  return (
    <Button
      variant="secondary"
      className="gap-2"
      onClick={() => void invoke("open_trailer", { url }).catch(() => undefined)}
    >
      <Clapperboard className="h-4 w-4" /> Trailer
    </Button>
  );
}
