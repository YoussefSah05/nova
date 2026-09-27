//! Command palette (⌘K / Ctrl+K). A glass overlay over the app that fuzzy-finds
//! content and exposes quick actions. Content search reuses the hybrid
//! (semantic + keyword) retrieval commands across all three types for the active
//! source; cmdk's own filtering is disabled (`shouldFilter={false}`) since the
//! backend already ranks. Selecting a result routes through the same `activate`
//! flow as a poster click. Mounted globally in `Home`.

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
} from "cmdk";
import {
  Clapperboard,
  Loader2,
  MonitorPlay,
  Plus,
  RotateCw,
  Settings,
  Tv,
  Users,
} from "lucide-react";
import type { CanonicalMediaSummary, ChannelGroup, SeriesSummary } from "@/types";
import { motion, AnimatePresence, useReducedMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";
import {
  channelGroupToItem,
  seriesToItem,
  movieToItem,
  type BrowseItem,
  type BrowseTab,
} from "./queries";
import { PosterFallback } from "./PosterFallback";
import { Artwork } from "@/components/Artwork";

export interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  profileId: string;
  sourceId: number | null;
  onActivate: (item: BrowseItem) => void;
  onGoToTab: (tab: BrowseTab) => void;
  onOpenSettings: () => void;
  onAddPlaylist: () => void;
  onSwitchProfile: () => void;
  onRefresh: () => void;
}

interface Results {
  live: BrowseItem[];
  movies: BrowseItem[];
  series: BrowseItem[];
}

const EMPTY: Results = { live: [], movies: [], series: [] };
const PER_TYPE = 6;

export function CommandPalette(props: CommandPaletteProps) {
  const { open, onOpenChange, profileId, sourceId, onActivate } = props;
  const reduce = useReducedMotion();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Results>(EMPTY);
  const [loading, setLoading] = useState(false);

  // Reset the query each time the palette opens.
  useEffect(() => {
    if (open) {
      setQuery("");
      setResults(EMPTY);
    }
  }, [open]);

  // Debounced hybrid search across all three content types for the active source.
  useEffect(() => {
    const q = query.trim();
    if (!open || sourceId === null || q.length < 2) {
      setResults(EMPTY);
      setLoading(false);
      return;
    }
    setLoading(true);
    let cancelled = false;
    const handle = window.setTimeout(async () => {
      const base = { profileId, sourceId, query: q, limit: PER_TYPE };
      const [live, movies, series] = await Promise.all([
        invoke<ChannelGroup[]>("semantic_search_channels", base).catch(() => []),
        invoke<CanonicalMediaSummary[]>("search_movies", base).catch(() => []),
        invoke<SeriesSummary[]>("semantic_search_series", base).catch(() => []),
      ]);
      if (cancelled) return;
      setResults({
        live: live.slice(0, PER_TYPE).map(channelGroupToItem),
        movies: movies.slice(0, PER_TYPE).map(movieToItem),
        series: series.slice(0, PER_TYPE).map(seriesToItem),
      });
      setLoading(false);
    }, 220);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
  }, [open, query, profileId, sourceId]);

  const actions = useMemo(() => buildActions(props), [props]);
  const q = query.trim().toLowerCase();
  const shownActions = q ? actions.filter((a) => a.keywords.some((k) => k.includes(q))) : actions;

  const choose = (item: BrowseItem) => {
    onOpenChange(false);
    onActivate(item);
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="fixed inset-0 z-[80] flex items-start justify-center p-4 pt-[12vh]"
          initial={reduce ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={reduce ? undefined : { opacity: 0 }}
          transition={{ duration: 0.12 }}
        >
          <button
            type="button"
            aria-label="Close command palette"
            className="absolute inset-0 cursor-default bg-black/60 backdrop-blur-sm"
            onClick={() => onOpenChange(false)}
          />
          <motion.div
            initial={reduce ? false : { opacity: 0, scale: 0.97, y: -8 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={reduce ? undefined : { opacity: 0, scale: 0.98, y: -8 }}
            transition={{ duration: 0.16, ease: [0.16, 1, 0.3, 1] }}
            className="nova-glass relative z-10 w-full max-w-xl overflow-hidden rounded-2xl"
          >
            <Command
              shouldFilter={false}
              loop
              onKeyDown={(e) => {
                if (e.key === "Escape") onOpenChange(false);
              }}
            >
              <div className="flex items-center gap-2 border-b border-white/10 px-4">
                {loading ? (
                  <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted-foreground" />
                ) : (
                  <SearchGlyph />
                )}
                <CommandInput
                  autoFocus
                  value={query}
                  onValueChange={setQuery}
                  placeholder="Search content or jump to…"
                  className="h-12 w-full bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
                />
              </div>
              <CommandList className="max-h-[min(60vh,420px)] overflow-y-auto p-2">
                <CommandEmpty className="px-3 py-8 text-center text-sm text-muted-foreground">
                  {q.length < 2 ? "Type to search your catalog." : "No matches."}
                </CommandEmpty>

                {shownActions.length > 0 && (
                  <Group heading="Actions">
                    {shownActions.map((a) => (
                      <Row
                        key={a.id}
                        value={a.id}
                        onSelect={() => {
                          onOpenChange(false);
                          a.run();
                        }}
                      >
                        <a.icon className="h-4 w-4 text-muted-foreground" />
                        <span>{a.label}</span>
                      </Row>
                    ))}
                  </Group>
                )}

                <ResultGroup heading="Live" items={results.live} onChoose={choose} />
                <ResultGroup heading="Movies" items={results.movies} onChoose={choose} />
                <ResultGroup heading="Series" items={results.series} onChoose={choose} />
              </CommandList>
            </Command>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

interface Action {
  id: string;
  label: string;
  icon: typeof Settings;
  keywords: string[];
  run: () => void;
}

function buildActions(p: CommandPaletteProps): Action[] {
  return [
    {
      id: "go-live",
      label: "Go to Live TV",
      icon: Tv,
      keywords: ["live", "tv", "channels"],
      run: () => p.onGoToTab("live"),
    },
    {
      id: "go-movies",
      label: "Go to Movies",
      icon: Clapperboard,
      keywords: ["movies", "films", "vod"],
      run: () => p.onGoToTab("movies"),
    },
    {
      id: "go-series",
      label: "Go to Series",
      icon: MonitorPlay,
      keywords: ["series", "shows", "tv"],
      run: () => p.onGoToTab("series"),
    },
    {
      id: "refresh",
      label: "Refresh playlist",
      icon: RotateCw,
      keywords: ["refresh", "reload", "update"],
      run: p.onRefresh,
    },
    {
      id: "add",
      label: "Add playlist",
      icon: Plus,
      keywords: ["add", "playlist", "import", "source"],
      run: p.onAddPlaylist,
    },
    {
      id: "settings",
      label: "Open Settings",
      icon: Settings,
      keywords: ["settings", "preferences", "appearance"],
      run: p.onOpenSettings,
    },
    {
      id: "switch-profile",
      label: "Switch profile",
      icon: Users,
      keywords: ["profile", "switch", "user", "account"],
      run: p.onSwitchProfile,
    },
  ];
}

function ResultGroup({
  heading,
  items,
  onChoose,
}: {
  heading: string;
  items: BrowseItem[];
  onChoose: (item: BrowseItem) => void;
}) {
  if (items.length === 0) return null;
  return (
    <>
      <CommandSeparator className="my-1 h-px bg-white/10" />
      <Group heading={heading}>
        {items.map((item) => (
          <Row key={item.key} value={item.key} onSelect={() => onChoose(item)}>
            <Thumb item={item} />
            <span className="min-w-0 flex-1">
              <span className="line-clamp-1">{item.title}</span>
              {item.subtitle && (
                <span className="line-clamp-1 text-xs text-muted-foreground">{item.subtitle}</span>
              )}
            </span>
          </Row>
        ))}
      </Group>
    </>
  );
}

function Thumb({ item }: { item: BrowseItem }) {
  const live = item.kind === "live";
  return (
    <span
      className={cn("flex h-9 shrink-0 overflow-hidden rounded-md bg-muted", live ? "w-14" : "w-6")}
    >
      <Artwork
        src={item.posterUrl}
        alt=""
        loading="lazy"
        decoding="async"
        className={cn("h-full w-full", live ? "object-contain p-0.5" : "object-cover")}
        fallback={<PosterFallback title={item.title} live={live} />}
      />
    </span>
  );
}

function Group({ heading, children }: { heading: string; children: React.ReactNode }) {
  return (
    <CommandGroup
      heading={heading}
      className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-muted-foreground"
    >
      {children}
    </CommandGroup>
  );
}

function Row({
  value,
  onSelect,
  children,
}: {
  value: string;
  onSelect: () => void;
  children: React.ReactNode;
}) {
  return (
    <CommandItem
      value={value}
      onSelect={onSelect}
      className="flex cursor-pointer select-none items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm text-foreground outline-none transition-colors data-[selected=true]:bg-white/10"
    >
      {children}
    </CommandItem>
  );
}

function SearchGlyph() {
  return (
    <svg
      className="h-4 w-4 shrink-0 text-muted-foreground"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="11" cy="11" r="8" />
      <path d="m21 21-4.3-4.3" />
    </svg>
  );
}
