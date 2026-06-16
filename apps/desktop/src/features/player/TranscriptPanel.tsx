//! Transcript companion panel (Phase 8).
//!
//! A right-side drawer synced to playback: shows the transcript (source + the
//! active translation when one exists), highlights and auto-scrolls the current
//! line, and lets you click a line to seek there. The search box runs an FTS5
//! search over the title's dialogue (source AND translation) and lists matches;
//! clicking a match seeks to it. Renders over the video — no mpv rect math, the
//! React UI already paints above the libmpv layer.

import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Search, X } from "lucide-react";

import { cn } from "@/lib/utils";
import type { Cue } from "./CaptionOverlay";

interface TranscriptLine {
  segmentId: number;
  startSec: number;
  endSec: number;
  text: string;
  translation: string | null;
}
interface SearchHit {
  segmentId: number;
  startSec: number;
  endSec: number;
  text: string;
  kind: string;
}

const MERGE_EPS = 0.05;

/// Prefer the persisted lines (they carry cached translations); append the live
/// cue tail that hasn't been queried back yet so the newest dialogue shows.
function mergeLive(
  lines: TranscriptLine[],
  sourceCues: Cue[],
  translatedCues: Cue[]
): TranscriptLine[] {
  const lastStart = lines.length ? lines[lines.length - 1].startSec : -Infinity;
  const tail = sourceCues
    .filter((c) => c.start > lastStart + MERGE_EPS)
    .map((c) => ({
      segmentId: -1,
      startSec: c.start,
      endSec: c.end,
      text: c.text,
      translation:
        translatedCues.find((t) => Math.abs(t.start - c.start) < MERGE_EPS)?.text ?? null,
    }));
  return [...lines, ...tail];
}

/// Split `text` around case-insensitive occurrences of `q`, wrapping matches.
function highlight(text: string, q: string): React.ReactNode {
  const needle = q.trim();
  if (!needle) return text;
  const out: React.ReactNode[] = [];
  const lower = text.toLowerCase();
  const ln = needle.toLowerCase();
  let i = 0;
  let k = 0;
  for (;;) {
    const at = lower.indexOf(ln, i);
    if (at < 0) {
      out.push(text.slice(i));
      break;
    }
    if (at > i) out.push(text.slice(i, at));
    out.push(
      <mark key={k++} className="rounded bg-[var(--color-accent)]/40 text-white">
        {text.slice(at, at + needle.length)}
      </mark>
    );
    i = at + needle.length;
  }
  return out;
}

export function TranscriptPanel({
  profileId,
  media,
  audioTrackIndex,
  targetLang,
  currentTime,
  sourceCues,
  translatedCues,
  onSeek,
  onClose,
}: {
  profileId: string;
  media: { kind: string; id: number };
  audioTrackIndex: number;
  targetLang: string;
  currentTime: number;
  sourceCues: Cue[];
  translatedCues: Cue[];
  onSeek: (sec: number) => void;
  onClose: () => void;
}) {
  const [lines, setLines] = useState<TranscriptLine[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[]>([]);
  const activeRef = useRef<HTMLButtonElement | null>(null);

  // Fetch the persisted transcript (re-fetch on track / target change).
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    void invoke<TranscriptLine[]>("get_transcript", {
      profileId,
      mediaKind: media.kind,
      mediaId: media.id,
      audioTrackIndex,
      targetLang: targetLang || null,
    })
      .then((l) => !cancelled && setLines(l))
      .catch(() => !cancelled && setLines([]))
      .finally(() => !cancelled && setLoading(false));
    return () => {
      cancelled = true;
    };
  }, [profileId, media.kind, media.id, audioTrackIndex, targetLang]);

  // Debounced FTS search over the title's dialogue.
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setHits([]);
      return;
    }
    const t = window.setTimeout(() => {
      void invoke<SearchHit[]>("search_transcript", {
        profileId,
        mediaKind: media.kind,
        mediaId: media.id,
        audioTrackIndex,
        query: q,
      })
        .then(setHits)
        .catch(() => setHits([]));
    }, 250);
    return () => window.clearTimeout(t);
  }, [query, profileId, media.kind, media.id, audioTrackIndex]);

  const merged = useMemo(
    () => mergeLive(lines, sourceCues, translatedCues),
    [lines, sourceCues, translatedCues]
  );
  const searching = query.trim().length > 0;

  // The line under the playhead (or the most recent one).
  const activeIdx = useMemo(() => {
    let idx = -1;
    for (let i = 0; i < merged.length; i++) {
      const l = merged[i];
      if (currentTime >= l.startSec && currentTime <= l.endSec) return i;
      if (l.startSec <= currentTime) idx = i;
    }
    return idx;
  }, [merged, currentTime]);

  // Keep the active line in view while not searching.
  useEffect(() => {
    if (searching) return;
    activeRef.current?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [activeIdx, searching]);

  return (
    <div className="nova-glass pointer-events-auto absolute right-0 top-0 bottom-0 z-40 flex w-[min(92vw,400px)] flex-col text-white">
      <div className="flex items-center justify-between gap-2 border-b border-white/10 px-4 py-3">
        <span className="text-sm font-semibold">Transcript</span>
        <button
          onClick={onClose}
          aria-label="Close transcript"
          className="flex h-7 w-7 items-center justify-center rounded-full text-white/70 hover:bg-white/10 hover:text-white"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="relative px-3 py-2">
        <Search className="pointer-events-none absolute left-5 top-1/2 h-4 w-4 -translate-y-1/2 text-white/40" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search dialogue"
          className="w-full rounded-md border border-white/10 bg-white/[0.04] py-1.5 pl-9 pr-3 text-sm text-white placeholder:text-white/40 outline-none focus:border-white/25"
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3 [scrollbar-width:thin]">
        {loading ? (
          <p className="px-3 py-8 text-center text-sm text-white/50">Loading transcript…</p>
        ) : searching ? (
          hits.length === 0 ? (
            <p className="px-3 py-8 text-center text-sm text-white/50">No matches.</p>
          ) : (
            <>
              <p className="px-3 py-1 text-[11px] uppercase tracking-wider text-white/45">
                {hits.length} match{hits.length === 1 ? "" : "es"}
              </p>
              {hits.map((h, i) => (
                <button
                  key={`${h.segmentId}-${h.kind}-${i}`}
                  onClick={() => onSeek(h.startSec)}
                  className="flex w-full flex-col gap-0.5 rounded-md px-3 py-2 text-left transition-colors hover:bg-white/10"
                >
                  <span className="flex items-center gap-2 text-[11px] text-white/40">
                    {formatTime(h.startSec)}
                    {h.kind === "translation" && (
                      <span className="rounded bg-white/10 px-1 py-px text-[10px]">translated</span>
                    )}
                  </span>
                  <span className="text-sm leading-snug text-white/90">
                    {highlight(h.text, query)}
                  </span>
                </button>
              ))}
            </>
          )
        ) : merged.length === 0 ? (
          <p className="px-3 py-8 text-center text-sm text-white/50">
            No transcript yet — enable captions to start transcribing.
          </p>
        ) : (
          merged.map((l, i) => {
            const active = i === activeIdx;
            return (
              <button
                key={`${l.segmentId}-${l.startSec}`}
                ref={active ? activeRef : null}
                onClick={() => onSeek(l.startSec)}
                className={cn(
                  "flex w-full flex-col gap-0.5 rounded-md px-3 py-1.5 text-left transition-colors hover:bg-white/10",
                  active && "bg-white/10"
                )}
              >
                <span className="text-[11px] text-white/35">{formatTime(l.startSec)}</span>
                <span
                  className={cn("text-sm leading-snug", active ? "text-white" : "text-white/80")}
                >
                  {l.text}
                </span>
                {l.translation && (
                  <span className="text-xs leading-snug text-[var(--color-accent-hover)]">
                    {l.translation}
                  </span>
                )}
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}

function formatTime(sec: number): string {
  const s = Math.max(0, Math.floor(sec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return `${h > 0 ? `${h}:` : ""}${mm}:${String(ss).padStart(2, "0")}`;
}
