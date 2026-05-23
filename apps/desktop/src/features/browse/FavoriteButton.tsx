import { useState } from "react";
import { HeartIcon } from "lucide-react";
import { cn } from "@/lib/utils";

interface FavoriteButtonProps {
  isFavorite: boolean;
  onToggle: (next: boolean) => Promise<void> | void;
  label: string;
  size?: "sm" | "md";
}

export function FavoriteButton({ isFavorite, onToggle, label, size = "md" }: FavoriteButtonProps) {
  const [pending, setPending] = useState(false);
  const sizeClass = size === "sm" ? "h-7 w-7" : "h-8 w-8";
  const iconClass = size === "sm" ? "h-3.5 w-3.5" : "h-4 w-4";

  async function handleClick(event: React.MouseEvent) {
    event.stopPropagation();
    if (pending) return;
    setPending(true);
    try {
      await onToggle(!isFavorite);
    } finally {
      setPending(false);
    }
  }

  return (
    <button
      type="button"
      onClick={handleClick}
      aria-pressed={isFavorite}
      aria-label={isFavorite ? `Remove ${label} from favourites` : `Add ${label} to favourites`}
      title={isFavorite ? "Remove from favourites" : "Add to favourites"}
      disabled={pending}
      className={cn(
        "flex flex-shrink-0 items-center justify-center rounded-full border transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-400 focus:ring-offset-2 focus:ring-offset-neutral-950 disabled:opacity-50",
        sizeClass,
        isFavorite
          ? "border-rose-400/40 bg-rose-500/15 text-rose-300 hover:bg-rose-500/25"
          : "border-white/10 bg-white/[0.03] text-white/40 hover:border-white/20 hover:text-white"
      )}
    >
      <HeartIcon className={cn(iconClass, isFavorite && "fill-current")} aria-hidden />
    </button>
  );
}
