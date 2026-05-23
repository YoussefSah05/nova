import { useState } from "react";
import { BookmarkIcon } from "lucide-react";
import { cn } from "@/lib/utils";

interface WatchlistButtonProps {
  inWatchlist: boolean;
  onToggle: (next: boolean) => Promise<void> | void;
  label: string;
  size?: "sm" | "md";
}

export function WatchlistButton({
  inWatchlist,
  onToggle,
  label,
  size = "md",
}: WatchlistButtonProps) {
  const [pending, setPending] = useState(false);
  const sizeClass = size === "sm" ? "h-7 w-7" : "h-8 w-8";
  const iconClass = size === "sm" ? "h-3.5 w-3.5" : "h-4 w-4";

  async function handleClick(event: React.MouseEvent) {
    event.stopPropagation();
    if (pending) return;
    setPending(true);
    try {
      await onToggle(!inWatchlist);
    } finally {
      setPending(false);
    }
  }

  return (
    <button
      type="button"
      onClick={handleClick}
      aria-pressed={inWatchlist}
      aria-label={inWatchlist ? `Remove ${label} from watchlist` : `Add ${label} to watchlist`}
      title={inWatchlist ? "Remove from watchlist" : "Save to watchlist"}
      disabled={pending}
      className={cn(
        "flex flex-shrink-0 items-center justify-center rounded-full border transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950 disabled:opacity-50",
        sizeClass,
        inWatchlist
          ? "border-indigo-400/40 bg-indigo-500/15 text-indigo-200 hover:bg-indigo-500/25"
          : "border-white/10 bg-white/[0.03] text-white/40 hover:border-white/20 hover:text-white"
      )}
    >
      <BookmarkIcon className={cn(iconClass, inWatchlist && "fill-current")} aria-hidden />
    </button>
  );
}
