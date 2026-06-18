//! A single profile tile on the picker. Glass surface in the app's identity, with
//! the avatar, name and — once the per-profile overview resolves — a "last watched"
//! hint, a resume title and at-a-glance stat badges. Each tile's hover glow takes
//! that profile's own saved accent colour so profiles feel individuated. In manage
//! mode it reveals rename / delete affordances instead of selecting on click.

import { Pencil, Trash2, PlayCircle } from "lucide-react";

import { cn } from "@/lib/utils";
import { motion, pressable } from "@/lib/motion";
import { accentTheme } from "@/lib/useAccentTheme";
import { formatRelative } from "@/lib/formatRelative";
import type { Profile, ProfileOverview } from "@/types";

interface ProfileCardProps {
  profile: Profile;
  overview?: ProfileOverview;
  managing?: boolean;
  onClick: (profile: Profile) => void;
  onEdit?: (profile: Profile) => void;
  onDelete?: (profile: Profile) => void;
  isDeleting?: boolean;
}

const compact = (n: number) =>
  new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(n);

export function ProfileCard({
  profile,
  overview,
  managing = false,
  onClick,
  onEdit,
  onDelete,
  isDeleting,
}: ProfileCardProps) {
  const initials = profile.name
    .split(" ")
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();

  const glow = accentTheme(overview?.accentColor ?? "amber").glowRgb;
  const lastWatched = profile.lastUsedAt
    ? `Watched ${formatRelative(profile.lastUsedAt)}`
    : "New profile";

  return (
    <motion.button
      {...pressable}
      type="button"
      onClick={() => (managing ? onEdit?.(profile) : onClick(profile))}
      style={
        {
          "--glow": `0 0 0 1px rgba(${glow},0.45), 0 14px 44px rgba(${glow},0.22)`,
        } as React.CSSProperties
      }
      className={cn(
        "nova-glass group relative flex w-44 flex-col items-center gap-3 rounded-2xl p-5 text-center",
        "transition-[transform,box-shadow,border-color] duration-200",
        "hover:-translate-y-1 hover:shadow-[var(--glow)] focus-visible:outline-none focus-visible:shadow-[var(--glow)]"
      )}
    >
      {/* Manage affordances. */}
      {managing && (
        <div className="absolute right-2 top-2 flex gap-1">
          <span
            role="button"
            tabIndex={-1}
            aria-label={`Edit ${profile.name}`}
            onClick={(e) => {
              e.stopPropagation();
              onEdit?.(profile);
            }}
            className="flex h-7 w-7 items-center justify-center rounded-full bg-white/10 text-white/80 transition-colors hover:bg-white/20"
          >
            <Pencil className="h-3.5 w-3.5" aria-hidden />
          </span>
          {onDelete && (
            <span
              role="button"
              tabIndex={-1}
              aria-label={`Delete ${profile.name}`}
              onClick={(e) => {
                e.stopPropagation();
                onDelete(profile);
              }}
              aria-disabled={isDeleting}
              className="flex h-7 w-7 items-center justify-center rounded-full bg-white/10 text-white/80 transition-colors hover:bg-rose-600 hover:text-white"
            >
              <Trash2 className="h-3.5 w-3.5" aria-hidden />
            </span>
          )}
        </div>
      )}

      <span
        className="flex h-16 w-16 items-center justify-center rounded-full text-xl font-semibold text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.25)] ring-2 ring-transparent transition-all group-hover:ring-white/30"
        style={{ backgroundColor: profile.avatarColor }}
      >
        {initials}
      </span>

      <div className="flex min-w-0 flex-col items-center gap-0.5">
        <span className="font-display max-w-full truncate text-base font-semibold text-white">
          {profile.name}
        </span>
        <span className="text-[11px] text-white/45">{lastWatched}</span>
      </div>

      {overview && (overview.lastTitle || overview.itemCount > 0) && (
        <div className="flex w-full flex-col items-center gap-1.5">
          {overview.lastTitle && (
            <span className="flex max-w-full items-center gap-1 text-[11px] text-primary/90">
              <PlayCircle className="h-3.5 w-3.5 shrink-0" aria-hidden />
              <span className="truncate">{overview.lastTitle}</span>
            </span>
          )}
          {overview.itemCount > 0 && (
            <div className="flex flex-wrap items-center justify-center gap-1">
              <StatBadge>{compact(overview.itemCount)} titles</StatBadge>
              {overview.sourceCount > 0 && (
                <StatBadge>
                  {overview.sourceCount} playlist{overview.sourceCount === 1 ? "" : "s"}
                </StatBadge>
              )}
            </div>
          )}
        </div>
      )}
    </motion.button>
  );
}

function StatBadge({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded-full bg-white/[0.06] px-2 py-0.5 text-[10px] font-medium text-white/55">
      {children}
    </span>
  );
}
