//! The floating top bar: sections, the ⌘K pill, the playlist switcher and
//! the profile menu. All of it drags the window (a deep drag region: its
//! buttons and menus stay clickable), it clears the traffic lights, and it
//! turns to glass once the screen under it scrolls.
//!
//! Both states are stacked layers whose opacity changes (compositor-only):
//! the blur itself is never switched on and off while scrolling.

import { memo, useRef } from "react";
import {
  ChevronLeftIcon,
  LogOutIcon,
  PlusIcon,
  RotateCwIcon,
  SearchIcon,
  SettingsIcon,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { SimpleSelect } from "@/components/ui/select";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SECTIONS, SECTION_LABELS, type SectionId } from "./navigation/nav";
import { useNav } from "./navigation/useNav";
import { useSession, useSessionStatus } from "./session";
import { useShell } from "./shell";

export const TOP_BAR_HEIGHT = 72;

export const TopBar = memo(function TopBar({ glass }: { glass: boolean }) {
  const nav = useNav();
  const session = useSession();
  const shell = useShell();
  const { sourceTotals, refreshing } = useSessionStatus();
  const { sources, activeSourceId } = session;
  const settingsHandoff = useRef(false);

  return (
    <header
      data-tauri-drag-region="deep"
      className="fixed inset-x-0 top-0 z-40 flex h-[72px] items-center gap-3 pl-[88px] pr-6 xl:gap-6"
    >
      <div
        aria-hidden
        className={cn(
          "nova-glass pointer-events-none absolute inset-0 rounded-none border-x-0 border-t-0 transition-opacity duration-200",
          glass ? "opacity-100" : "opacity-0"
        )}
      />
      <div
        aria-hidden
        className={cn(
          "pointer-events-none absolute inset-0 bg-gradient-to-b from-black/55 to-transparent transition-opacity duration-200",
          glass ? "opacity-0" : "opacity-100"
        )}
      />

      <div className="relative flex shrink-0 items-center gap-2">
        <button
          type="button"
          onClick={() => nav.switchTo("home")}
          aria-label="Nova home"
          className="flex items-center gap-2.5 rounded-lg focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span className="flex h-[30px] w-[30px] items-center justify-center rounded-[9px] bg-gradient-to-br from-[var(--color-brand-hi)] to-[var(--color-brand-lo)] font-display text-lg font-medium text-primary-foreground shadow-[0_6px_20px_rgba(224,163,62,.25)]">
            N
          </span>
        </button>
        {nav.canGoBack && (
          <button
            type="button"
            onClick={nav.back}
            aria-label="Back"
            className="flex h-9 w-9 items-center justify-center rounded-[10px] text-white/70 transition-colors hover:bg-white/[0.06] hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <ChevronLeftIcon className="h-5 w-5" strokeWidth={1.5} aria-hidden />
          </button>
        )}
      </div>

      <nav aria-label="Sections" className="relative flex shrink-0 items-center gap-1">
        {SECTIONS.map((id) => (
          <SectionButton
            key={id}
            id={id}
            active={nav.state.section === id}
            onSelect={nav.switchTo}
          />
        ))}
      </nav>

      <div className="flex-1" />

      <button
        type="button"
        onClick={() => shell.setPaletteOpen(true)}
        aria-label="Search, or tell Nova what you want"
        className="relative flex h-[38px] w-[320px] min-w-0 items-center gap-2.5 rounded-xl border border-white/[0.12] bg-white/[0.05] pl-3.5 pr-2 text-sm text-white/60 transition-colors hover:border-white/20 hover:bg-white/[0.08] focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <SearchIcon className="h-4 w-4 shrink-0" strokeWidth={1.5} aria-hidden />
        <span className="min-w-0 flex-1 truncate text-left">Search titles, channels…</span>
        <kbd className="shrink-0 rounded-md border border-white/[0.14] px-1.5 py-0.5 font-sans text-xs text-white/70">
          ⌘K
        </kbd>
      </button>

      {sources.length > 0 && activeSourceId !== null && (
        <SimpleSelect
          value={String(activeSourceId)}
          options={sources.map((s) => ({
            value: String(s.id),
            label:
              sourceTotals[s.id] !== undefined
                ? `${s.name} (${new Intl.NumberFormat().format(sourceTotals[s.id])})`
                : s.name,
          }))}
          onChange={(v) => session.setActiveSourceId(Number(v))}
          ariaLabel="Playlist"
          className="relative h-[38px] w-40 shrink-0 rounded-xl border-white/10 bg-white/[0.04] [&>span]:truncate"
        />
      )}

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label="Profile and settings"
            className="relative flex h-[38px] w-[38px] shrink-0 items-center justify-center rounded-full focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <span
              className="flex h-[30px] w-[30px] items-center justify-center rounded-full text-xs font-medium text-white shadow-[inset_0_0_0_1px_rgba(255,255,255,.14)]"
              style={{ background: session.profile.avatarColor }}
            >
              {session.profile.name.slice(0, 1).toUpperCase()}
            </span>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          className="w-56"
          onCloseAutoFocus={(event) => {
            if (settingsHandoff.current) {
              event.preventDefault();
              settingsHandoff.current = false;
              shell.openSettings();
            }
          }}
        >
          <DropdownMenuLabel>{session.profile.name}</DropdownMenuLabel>
          <DropdownMenuItem
            onSelect={() => {
              settingsHandoff.current = true;
            }}
          >
            <SettingsIcon className="h-4 w-4" strokeWidth={1.5} aria-hidden /> Settings
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => void session.refresh()}
            disabled={activeSourceId === null || refreshing}
          >
            <RotateCwIcon
              className={cn("h-4 w-4", refreshing && "animate-spin")}
              strokeWidth={1.5}
              aria-hidden
            />
            {refreshing ? "Refreshing playlist…" : "Refresh playlist"}
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={shell.openAddSource}>
            <PlusIcon className="h-4 w-4" strokeWidth={1.5} aria-hidden /> Add playlist
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={session.switchProfile}>
            <LogOutIcon className="h-4 w-4" strokeWidth={1.5} aria-hidden /> Switch profile
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </header>
  );
});

const SectionButton = memo(function SectionButton({
  id,
  active,
  onSelect,
}: {
  id: SectionId;
  active: boolean;
  onSelect: (id: SectionId) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onSelect(id)}
      aria-current={active ? "page" : undefined}
      className={cn(
        "relative flex h-9 items-center rounded-[10px] px-3.5 text-sm font-medium transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active ? "text-white" : "text-white/60 hover:bg-white/[0.06] hover:text-white"
      )}
    >
      {SECTION_LABELS[id]}
      {active && (
        <span
          aria-hidden
          className="absolute bottom-[3px] left-1/2 h-0.5 w-4 -translate-x-1/2 rounded-full bg-primary"
        />
      )}
    </button>
  );
});
