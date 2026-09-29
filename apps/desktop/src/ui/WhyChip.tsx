//! Why a set of titles is shown ("Because you watched…"). Neutral, never
//! amber: it explains Nova's choice, it is not an action. Later the local
//! personalization model fills the reason; today it is the factual source.

import { InfoIcon } from "lucide-react";

export function WhyChip({ reason }: { reason: string }) {
  return (
    <span
      className="flex h-7 items-center gap-1.5 rounded-full border border-white/[0.14] bg-white/[0.04] px-2.5 text-xs text-white/70"
      title={reason}
    >
      <InfoIcon className="h-[13px] w-[13px]" strokeWidth={1.5} aria-hidden />
      <span className="sr-only">Why these titles: </span>
      {reason}
    </span>
  );
}
