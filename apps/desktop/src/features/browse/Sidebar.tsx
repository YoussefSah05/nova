//! Left navigation rail (TV-app style): the Live / Movies / Series content
//! switch with an animated brand active-indicator + per-type counts, plus
//! Settings and Profile pinned at the bottom. Replaces the old top tabs +
//! header chrome.

import { Tv, Clapperboard, MonitorPlay, Settings } from "lucide-react";

import { cn } from "@/lib/utils";
import { motion, useReducedMotion } from "@/lib/motion";
import type { Profile } from "@/types";
import type { BrowseTab } from "./queries";

const NAV: { tab: BrowseTab; label: string; Icon: typeof Tv }[] = [
  { tab: "live", label: "Live", Icon: Tv },
  { tab: "movies", label: "Movies", Icon: Clapperboard },
  { tab: "series", label: "Series", Icon: MonitorPlay },
];

const compact = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });

export function Sidebar({
  tab,
  onTabChange,
  counts,
  profile,
  onSettings,
  onSwitchProfile,
}: {
  tab: BrowseTab;
  onTabChange: (tab: BrowseTab) => void;
  counts: Record<BrowseTab, number>;
  profile: Profile;
  onSettings: () => void;
  onSwitchProfile: () => void;
}) {
  const reduce = useReducedMotion();

  return (
    <aside className="nova-glass flex w-[84px] shrink-0 flex-col items-stretch py-4">
      <span className="font-display mb-4 text-center text-2xl font-bold text-primary">N</span>

      <nav className="flex flex-1 flex-col gap-1 px-2">
        {NAV.map(({ tab: t, label, Icon }) => {
          const active = tab === t;
          return (
            <button
              key={t}
              onClick={() => onTabChange(t)}
              aria-current={active ? "page" : undefined}
              aria-label={label}
              className={cn(
                "group relative flex flex-col items-center gap-1 rounded-xl py-2.5 text-[11px] font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                active ? "bg-white/[0.06] text-primary" : "text-white/55 hover:text-white"
              )}
            >
              {active &&
                (reduce ? (
                  <span className="absolute left-0 top-1/2 h-7 w-0.5 -translate-y-1/2 rounded-full bg-primary" />
                ) : (
                  <motion.span
                    layoutId="rail-active"
                    className="absolute left-0 top-1/2 h-7 w-0.5 -translate-y-1/2 rounded-full bg-primary"
                    transition={{ type: "spring", stiffness: 500, damping: 36 }}
                  />
                ))}
              <Icon className="h-5 w-5" aria-hidden />
              <span>{label}</span>
              <span className="text-[10px] tabular-nums text-white/35">
                {compact.format(counts[t])}
              </span>
            </button>
          );
        })}
      </nav>

      <div className="mt-2 flex flex-col items-center gap-2">
        <button
          onClick={onSettings}
          aria-label="Settings"
          title="Settings"
          className="flex h-9 w-9 items-center justify-center rounded-full text-white/55 transition-colors hover:bg-white/10 hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Settings className="h-5 w-5" aria-hidden />
        </button>
        <button
          onClick={onSwitchProfile}
          aria-label="Switch profile"
          title={profile.name}
          className="flex h-9 w-9 items-center justify-center rounded-full text-xs font-semibold text-white ring-2 ring-transparent transition-all hover:ring-primary hover:[box-shadow:var(--shadow-brand-glow)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          style={{ backgroundColor: profile.avatarColor }}
        >
          {profile.name.slice(0, 2).toUpperCase()}
        </button>
      </div>
    </aside>
  );
}
