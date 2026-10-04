import { useState } from "react";
import {
  ArrowUp,
  Bookmark,
  BookmarkPlus,
  Cpu,
  Focus,
  Orbit,
  Play,
  Search,
  ShieldCheck,
  Sparkles,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { clockTime } from "@/sections/home/homeFeed";
import type { Cue } from "./CaptionOverlay";
import type { PlayMedia } from "./PlayerModal";
import { TranscriptPanel } from "./TranscriptPanel";

export type CompanionTab = "now" | "find" | "explore" | "saved";
const SPACES = [
  { key: "now", label: "Now", icon: Focus },
  { key: "find", label: "Find", icon: Search },
  { key: "explore", label: "Explore", icon: Orbit },
  { key: "saved", label: "Saved", icon: Bookmark },
] as const;

/** Frontend capability boundary: no simulated AI answers or new model jobs.
 * Session notes deliberately live here, outside the conditional open panel. */
export function PlayerCompanion({
  tab,
  onTab,
  onClose,
  profileId,
  media,
  currentTime,
  audioTrackIndex,
  sourceCues,
  captionNote,
  onSeek,
  onEnableCaptions,
  onBrowse,
}: {
  tab: CompanionTab | null;
  onTab: (tab: CompanionTab) => void;
  onClose: () => void;
  profileId: string;
  media: PlayMedia;
  currentTime: number;
  audioTrackIndex: number;
  sourceCues: Cue[];
  captionNote: string | null;
  onSeek: (time: number) => void;
  onEnableCaptions: () => void;
  onBrowse: () => void;
}) {
  const [scope, setScope] = useState<"watched" | "whole">("watched");
  const [finder, setFinder] = useState<"dialogue" | "moments" | "chapters">("dialogue");
  const [activity, setActivity] = useState(false);
  const [notes, setNotes] = useState<{ id: number; time: number; text: string }[]>([]);
  const isLive = media.kind === "live";
  const currentCue = sourceCues.find((cue) => currentTime >= cue.start && currentTime <= cue.end);
  const available = sourceCues.filter((cue) => scope === "whole" || cue.start <= currentTime);
  function saveMoment() {
    const time = Math.floor(currentTime);
    setNotes((previous) =>
      previous.some((note) => note.time === time)
        ? previous
        : [...previous, { id: time, time, text: "" }]
    );
    onTab("saved");
  }
  if (!tab) return null;

  return (
    <aside
      aria-label="Nova companion"
      data-controls
      className="nova-panel-enter flex min-h-0 w-[352px] max-w-[42vw] shrink-0 flex-col border-l border-border bg-card text-foreground [&_svg]:stroke-[1.5]"
    >
      <header className="flex items-center justify-between gap-2 p-5 pb-4">
        <div>
          <p className="mb-2 flex items-center gap-2 text-xs tracking-widest text-primary">
            <Sparkles className="h-4 w-4" aria-hidden /> NOVA
          </p>
          <h2 className="font-display text-xl font-medium">Watching, understood.</h2>
        </div>
        <Button variant="ghost" size="iconSm" aria-label="Close companion" onClick={onClose}>
          <X />
        </Button>
      </header>
      <nav aria-label="Companion spaces" className="mx-5 grid grid-cols-4 border-b border-border">
        {SPACES.map(({ key, label, icon: Icon }) => (
          <button
            key={key}
            type="button"
            aria-pressed={tab === key}
            onClick={() => {
              setActivity(false);
              onTab(key);
            }}
            className={cn(
              "flex flex-col items-center gap-2 border-b-2 px-1 py-3 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              tab === key
                ? "border-primary text-primary"
                : "border-transparent text-muted-foreground hover:text-foreground"
            )}
          >
            <Icon className="h-4 w-4" aria-hidden />
            {label}
          </button>
        ))}
      </nav>
      <div className="min-h-0 flex-1 overflow-y-auto p-5 [scrollbar-width:thin]">
        <div key={activity ? "activity" : tab} className="nova-fade-in flex h-full flex-col gap-4">
          {activity ? (
            <>
              <h3 className="text-base font-medium">On this device</h3>
              <p className="text-sm text-muted-foreground">Playback keeps priority.</p>
              <Activity
                label="Source captions"
                value={captionNote ?? (sourceCues.length ? "Available" : "Off")}
              />
              <Activity label="Scene understanding" value="Coming later" />
              <Activity label="Translation" value="Coming later" />
              <p className="text-xs leading-relaxed text-muted-foreground">
                Questions, explanations and generated moments will appear here when the intelligence
                upgrade is connected.
              </p>
              <Button variant="outline" onClick={() => setActivity(false)}>
                Back to companion
              </Button>
            </>
          ) : tab === "now" ? (
            <>
              {!isLive && (
                <label className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
                  <span className="flex items-center gap-1">
                    <ShieldCheck className="h-4 w-4" aria-hidden /> Spoiler boundary
                  </span>
                  <select
                    aria-label="Spoiler boundary"
                    value={scope}
                    onChange={(event) => setScope(event.target.value as "watched" | "whole")}
                    className="max-w-40 rounded-md border border-border bg-secondary p-1 text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    <option value="watched">Up to {clockTime(currentTime)}</option>
                    <option value="whole">Whole title · spoilers</option>
                  </select>
                </label>
              )}
              <div className="flex flex-col gap-3 rounded-xl border border-primary/20 bg-primary/5 p-4">
                <p className="text-xs text-primary">
                  {isLive ? "Live now" : `At this moment · ${clockTime(currentTime)}`}
                </p>
                <h3 className="font-display text-lg font-medium">{media.title}</h3>
                {currentCue && !isLive ? (
                  <>
                    <p className="text-sm leading-relaxed">“{currentCue.text}”</p>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="self-start px-0 text-primary"
                      onClick={() => onSeek(currentCue.start)}
                    >
                      <Play />
                      {clockTime(currentCue.start)} · Source dialogue
                    </Button>
                  </>
                ) : (
                  <p className="text-sm leading-relaxed text-muted-foreground">
                    {isLive
                      ? "Open the guide to see this programme and what’s next."
                      : "Source dialogue will appear here as captions become available."}
                  </p>
                )}
              </div>
              {!isLive && (
                <div className="grid grid-cols-2 gap-2">
                  <Button variant="secondary" size="sm" onClick={() => onTab("find")}>
                    <Search /> Find dialogue
                  </Button>
                  <Button variant="secondary" size="sm" onClick={saveMoment}>
                    <BookmarkPlus /> Save moment
                  </Button>
                </div>
              )}
              <Upcoming
                title="A little more context"
                description="Catch up, explain this scene and ask about what you’re watching. Answers will link to source dialogue and respect your spoiler boundary."
              />
              <Button variant="outline" onClick={isLive ? onBrowse : onEnableCaptions}>
                {isLive ? "Open live guide" : "Enable source captions"}
              </Button>
              {!isLive && available.length > 0 && (
                <p className="text-xs text-muted-foreground">
                  {available.length} source lines available in this playback session. More saved
                  dialogue may be available in Find.
                </p>
              )}
            </>
          ) : tab === "find" ? (
            isLive ? (
              <Upcoming
                title="Live dialogue is not available yet"
                description="Live moment search needs captured speech. You can still explore the channel guide."
                action={
                  <Button variant="outline" onClick={onBrowse}>
                    Open live guide
                  </Button>
                }
              />
            ) : (
              <>
                <div className="flex gap-1" aria-label="Find mode">
                  {(["dialogue", "moments", "chapters"] as const).map((value) => (
                    <Button
                      key={value}
                      variant={finder === value ? "secondary" : "ghost"}
                      size="sm"
                      aria-pressed={finder === value}
                      onClick={() => setFinder(value)}
                      className="px-3 capitalize"
                    >
                      {value}
                    </Button>
                  ))}
                </div>
                {finder === "dialogue" ? (
                  <TranscriptPanel
                    embedded
                    profileId={profileId}
                    media={media}
                    audioTrackIndex={audioTrackIndex}
                    currentTime={currentTime}
                    sourceCues={sourceCues}
                    maxTime={scope === "watched" ? currentTime : undefined}
                    onSeek={onSeek}
                    onClose={onClose}
                  />
                ) : (
                  <Upcoming
                    title={
                      finder === "moments"
                        ? "Find a scene by meaning"
                        : "Chapters from your dialogue"
                    }
                    description={
                      finder === "moments"
                        ? "Describe a moment, concept or theme. Results will include the matching dialogue, source and a playable timestamp."
                        : "Suggested chapters will help you navigate covered scenes. They will stay separate from official chapters."
                    }
                    action={
                      <Button variant="outline" onClick={() => setFinder("dialogue")}>
                        Search available dialogue
                      </Button>
                    }
                  />
                )}
              </>
            )
          ) : tab === "explore" ? (
            <>
              <p className="text-xs uppercase tracking-widest text-primary">
                {isLive ? "Live TV" : media.kind === "episode" ? "Series" : "Film"}
              </p>
              <h3 className="font-display text-xl font-medium">{media.title}</h3>
              {media.subtitle && <p className="text-sm text-muted-foreground">{media.subtitle}</p>}
              <Button variant="secondary" onClick={onBrowse}>
                {isLive
                  ? "Channels & guide"
                  : media.kind === "episode"
                    ? "Seasons & episodes"
                    : "Title information"}
              </Button>
              <Upcoming
                title="Follow your curiosity"
                description="People, themes, important moments and recommendations from your library will live here. Generated interpretations will stay distinct from catalog facts."
              />
            </>
          ) : (
            <>
              <div>
                <h3 className="text-base font-medium">Your moments</h3>
                <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                  Notes stay here while this player is open. Saving across sessions is coming later.
                </p>
              </div>
              {isLive ? (
                <p className="text-sm text-muted-foreground">
                  Live bookmarks need replay support. Save moments from a film or episode instead.
                </p>
              ) : (
                <>
                  <Button variant="secondary" onClick={saveMoment}>
                    <BookmarkPlus />
                    Save {clockTime(currentTime)}
                  </Button>
                  {notes.length === 0 && (
                    <p className="text-sm text-muted-foreground">
                      Keep a line or a moment worth returning to.
                    </p>
                  )}
                  {notes.map((note) => (
                    <div key={note.id} className="space-y-3 border-b border-border pb-4">
                      <div className="flex justify-between">
                        <Button
                          variant="ghost"
                          size="sm"
                          className="text-primary"
                          onClick={() => onSeek(note.time)}
                        >
                          <Play />
                          {clockTime(note.time)}
                        </Button>
                        <Button
                          variant="ghost"
                          size="iconSm"
                          aria-label={`Remove moment ${clockTime(note.time)}`}
                          onClick={() =>
                            setNotes((values) => values.filter((value) => value.id !== note.id))
                          }
                        >
                          <X />
                        </Button>
                      </div>
                      <label className="flex flex-col gap-2 text-xs text-muted-foreground">
                        Your note
                        <textarea
                          maxLength={2000}
                          value={note.text}
                          onChange={(event) =>
                            setNotes((values) =>
                              values.map((value) =>
                                value.id === note.id
                                  ? { ...value, text: event.target.value }
                                  : value
                              )
                            )
                          }
                          placeholder="What made this worth keeping?"
                          className="min-h-20 resize-y rounded-lg border border-border bg-secondary p-3 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        />
                      </label>
                    </div>
                  ))}
                </>
              )}
            </>
          )}
        </div>
      </div>
      {tab === "now" && !activity && (
        <div className="mx-5 mb-4 flex items-center gap-2 rounded-xl border border-border bg-secondary p-3 text-xs text-muted-foreground">
          <span className="flex-1">Ask Nova · coming with intelligence</span>
          <ArrowUp className="h-4 w-4" aria-hidden />
        </div>
      )}
      <footer className="flex items-center justify-between border-t border-border p-4 text-xs text-muted-foreground">
        <button
          type="button"
          onClick={() => setActivity((value) => !value)}
          className="flex items-center gap-2 rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <Cpu className="h-4 w-4" aria-hidden /> On this device
        </button>
        <span>Private by design</span>
      </footer>
    </aside>
  );
}

function Upcoming({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-3">
      <p className="text-xs text-primary">Intelligence · coming later</p>
      <h3 className="text-base font-medium">{title}</h3>
      <p className="text-sm leading-relaxed text-muted-foreground">{description}</p>
      {action}
    </div>
  );
}
function Activity({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-3 border-b border-border pb-3 text-sm">
      <span>{label}</span>
      <span className="text-right text-xs text-muted-foreground">{value}</span>
    </div>
  );
}
