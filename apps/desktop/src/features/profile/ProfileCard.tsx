//! A profile on the launch screen. Glass card in the shell's identity: avatar,
//! name, when it was last used, what it was watching (amber — the profile's
//! own) and its library at a glance. The hover glow takes the profile's saved
//! accent. In manage mode, edit and delete are separate buttons beside it.

import { Pencil, Trash2 } from "lucide-react";

import { cn } from "@/lib/utils";
import { motion, pressable } from "@/lib/motion";
import { accentTheme } from "@/lib/useAccentTheme";
import { formatRelative } from "@/lib/formatRelative";
import type { Profile, ProfileOverview } from "@/types";
import { initialsOf } from "./launch";

interface ProfileCardProps {
  profile: Profile;
  overview?: ProfileOverview;
  managing?: boolean;
  opening?: boolean;
  dimmed?: boolean;
  onClick: (profile: Profile) => void;
  onEdit?: (profile: Profile) => void;
  onDelete?: (profile: Profile) => void;
  isDeleting?: boolean;
}

const compact = (n: number) =>
  new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(n);

function libraryLine(overview: ProfileOverview): string {
  if (overview.sourceCount === 0) return "No playlists yet";
  const playlists = `${overview.sourceCount} playlist${overview.sourceCount === 1 ? "" : "s"}`;
  return overview.itemCount > 0
    ? `${compact(overview.itemCount)} titles · ${playlists}`
    : playlists;
}

export function ProfileCard({
  profile,
  overview,
  managing = false,
  opening = false,
  dimmed = false,
  onClick,
  onEdit,
  onDelete,
  isDeleting,
}: ProfileCardProps) {
  const glow = accentTheme(overview?.accentColor ?? "amber").glowRgb;
  const hint = opening
    ? "Opening…"
    : profile.lastUsedAt
      ? `Watched ${formatRelative(profile.lastUsedAt)}`
      : "New profile";

  return (
    <div
      inert={dimmed || undefined}
      className={cn(
        "group relative h-full transition-opacity duration-200",
        dimmed && "pointer-events-none opacity-40"
      )}
    >
      <motion.button
        {...pressable}
        type="button"
        aria-busy={opening || undefined}
        onClick={() => (managing ? onEdit?.(profile) : onClick(profile))}
        style={
          {
            "--glow": `0 0 0 1px rgba(${glow},0.45), 0 18px 48px rgba(${glow},0.2)`,
          } as React.CSSProperties
        }
        className={cn(
          "nova-glass relative flex h-full w-[232px] flex-col gap-4 overflow-hidden rounded-2xl p-5 text-left",
          "transition-[transform,box-shadow] duration-200",
          "hover:-translate-y-1 hover:shadow-[var(--glow)] focus-visible:shadow-[var(--glow)] focus-visible:outline-none",
          opening && "shadow-[var(--glow)]"
        )}
      >
        <span
          className="flex h-12 w-12 items-center justify-center rounded-[14px] text-base font-medium text-white shadow-[inset_0_1px_0_rgba(255,255,255,0.25)]"
          style={{ backgroundColor: profile.avatarColor }}
          aria-hidden
        >
          {initialsOf(profile.name)}
        </span>

        <span className="flex min-w-0 flex-col gap-1">
          <span className="font-display truncate text-lg font-medium text-white">
            {profile.name}
          </span>
          <span className="text-xs text-white/50">{hint}</span>
        </span>

        <span className="flex min-h-[64px] flex-col justify-end gap-1 border-t border-white/[0.08] pt-3">
          {overview ? (
            <>
              {overview.lastTitle ? (
                <span className="flex min-w-0 flex-col">
                  <span className="text-xs text-white/45">Continue</span>
                  <span className="truncate text-sm text-primary">{overview.lastTitle}</span>
                </span>
              ) : (
                <span className="text-sm text-white/45">Nothing in progress</span>
              )}
              <span className="text-xs text-white/50">{libraryLine(overview)}</span>
            </>
          ) : (
            <span className="nova-shimmer h-4 w-3/4 rounded bg-white/[0.06]" aria-hidden />
          )}
        </span>

        {opening && (
          <span
            aria-hidden
            className="nova-shimmer absolute inset-x-0 bottom-0 h-0.5 bg-[var(--color-brand)]/40"
          />
        )}
      </motion.button>

      {managing && (
        <div className="absolute right-3 top-3 flex gap-1">
          <button
            type="button"
            aria-label={`Edit ${profile.name}`}
            onClick={() => onEdit?.(profile)}
            className="flex h-8 w-8 items-center justify-center rounded-full bg-white/10 text-white/80 transition-colors hover:bg-white/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <Pencil className="h-3.5 w-3.5" strokeWidth={1.5} aria-hidden />
          </button>
          {onDelete && (
            <button
              type="button"
              aria-label={`Delete ${profile.name}`}
              disabled={isDeleting}
              onClick={() => onDelete(profile)}
              className="flex h-8 w-8 items-center justify-center rounded-full bg-white/10 text-white/80 transition-colors hover:bg-rose-600 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
            >
              <Trash2 className="h-3.5 w-3.5" strokeWidth={1.5} aria-hidden />
            </button>
          )}
        </div>
      )}
    </div>
  );
}
