//! The app once a profile is chosen: the floating top bar over full-bleed
//! screens, one navigation stack per section.
//!
//! Every screen a section has opened stays mounted (hidden when not on top),
//! so switching sections and going back are instant and land exactly where
//! they were. Sections are mounted the first time they are visited.

import { memo, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { AlertCircleIcon, XIcon } from "lucide-react";

import type { Profile } from "@/types";
import { BrowseBoundary } from "@/features/browse/BrowseBoundary";
import { CommandPalette } from "@/features/browse/CommandPalette";
import { HomeSection } from "@/sections/home/HomeSection";
import { LibrarySection } from "@/sections/library/LibrarySection";
import { MoviesSection } from "@/sections/movies/MoviesSection";
import { SeriesSection } from "@/sections/series/SeriesSection";
import { LiveSection } from "@/sections/live/LiveSection";
import { SeriesPage } from "@/pages/SeriesPage";
import { TitlePage } from "@/pages/TitlePage";
import { SECTIONS, SECTION_LABELS, type Entry, type SectionId } from "./navigation/nav";
import { NavProvider } from "./navigation/NavProvider";
import { useNav } from "./navigation/useNav";
import { PlaybackProvider } from "./PlaybackProvider";
import { Screen } from "./Screen";
import { SessionProvider } from "./SessionProvider";
import { useSession, useSessionStatus } from "./session";
import { ShellProvider } from "./ShellProvider";
import { useShell } from "./shell";
import { TopBar } from "./TopBar";
import { useActivate } from "./useActivate";

export function AppShell({
  profile,
  onSwitchProfile,
}: {
  profile: Profile;
  onSwitchProfile: () => void;
}) {
  return (
    <SessionProvider profile={profile} onSwitchProfile={onSwitchProfile}>
      <PlaybackProvider>
        <ShellProvider>
          <Navigation />
        </ShellProvider>
      </PlaybackProvider>
    </SessionProvider>
  );
}

function Navigation() {
  const { overlayOpen } = useShell();
  const overlay = useRef(overlayOpen);
  overlay.current = overlayOpen;
  const blocked = useCallback(() => overlay.current, []);
  return (
    <NavProvider blocked={blocked}>
      <Frame />
    </NavProvider>
  );
}

function Frame() {
  const nav = useNav();
  const { section } = nav.state;
  const [visited, setVisited] = useState<ReadonlySet<SectionId>>(() => new Set([section]));
  const [glass, setGlass] = useState(false);

  useEffect(() => {
    setVisited((v) => (v.has(section) ? v : new Set([...v, section])));
  }, [section]);

  // Pages belong to the playlist they were opened from.
  const { activeSourceId } = useSession();
  const shownSource = useRef(activeSourceId);
  const { resetAll } = nav;
  useEffect(() => {
    if (shownSource.current !== null && shownSource.current !== activeSourceId) resetAll();
    shownSource.current = activeSourceId;
  }, [activeSourceId, resetAll]);

  return (
    <div className="relative h-screen overflow-hidden bg-background text-foreground">
      <TopBar glass={glass} />
      {/* One flat, keyed list: a section visited later never shifts the
          screens of the others (which would remount them). */}
      {SECTIONS.filter((s) => visited.has(s)).flatMap((s) => {
        const stack = nav.state.stacks[s];
        return stack.map((entry, i) => {
          const active = s === section && i === stack.length - 1;
          return (
            <Screen
              key={entry.key}
              active={active}
              label={screenLabel(s, entry)}
              onScrolledChange={setGlass}
            >
              <ScreenBody section={s} entry={entry} />
            </Screen>
          );
        });
      })}
      <ErrorBanner />
      <Palette />
    </div>
  );
}

function screenLabel(section: SectionId, entry: Entry): string {
  switch (entry.route.kind) {
    case "root":
      return SECTION_LABELS[section];
    case "title":
      return entry.route.item.title;
    case "series":
      return entry.route.series.displayTitle;
  }
}

/// A screen's content. Memoized on its route: navigating elsewhere never
/// re-renders the screens that stay mounted underneath.
const ScreenBody = memo(function ScreenBody({
  section,
  entry,
}: {
  section: SectionId;
  entry: Entry;
}) {
  return (
    <BrowseBoundary resetKey={entry.key}>
      <ScreenContent section={section} entry={entry} />
    </BrowseBoundary>
  );
});

function ScreenContent({ section, entry }: { section: SectionId; entry: Entry }): ReactNode {
  const route = entry.route;
  switch (route.kind) {
    case "title":
      return <TitlePage item={route.item} />;
    case "series":
      return <SeriesPage series={route.series} />;
    case "root":
      switch (section) {
        case "home":
          return <HomeSection />;
        case "library":
          return <LibrarySection />;
        case "movies":
          return <MoviesSection />;
        case "series":
          return <SeriesSection />;
        case "live":
          return <LiveSection />;
      }
  }
}

/// The session's last error, dismissable, over every screen.
function ErrorBanner() {
  const { setError } = useSession();
  const { error } = useSessionStatus();
  if (!error) return null;
  return (
    <div
      role="alert"
      className="nova-glass fixed bottom-6 left-1/2 z-40 flex max-w-xl -translate-x-1/2 items-start gap-3 rounded-2xl px-4 py-3 text-sm"
    >
      <AlertCircleIcon
        className="mt-0.5 h-4 w-4 shrink-0 text-destructive"
        strokeWidth={1.5}
        aria-hidden
      />
      <span className="flex-1 text-white/85">{error}</span>
      <button
        type="button"
        onClick={() => setError(null)}
        aria-label="Dismiss"
        className="rounded-md text-white/60 transition-colors hover:text-white focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <XIcon className="h-4 w-4" strokeWidth={1.5} aria-hidden />
      </button>
    </div>
  );
}

function Palette() {
  const session = useSession();
  const shell = useShell();
  const nav = useNav();
  const activate = useActivate();
  return (
    <CommandPalette
      open={shell.paletteOpen}
      onOpenChange={shell.setPaletteOpen}
      profileId={session.profile.id}
      sourceId={session.activeSourceId}
      onActivate={activate}
      onGoToTab={nav.switchTo}
      onOpenSettings={shell.openSettings}
      onAddPlaylist={shell.openAddSource}
      onSwitchProfile={session.switchProfile}
      onRefresh={() => void session.refresh()}
    />
  );
}
