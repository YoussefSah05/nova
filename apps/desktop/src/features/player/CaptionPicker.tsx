//! Captions menu — the popover behind the CC button.
//!
//! Display mode (off / source / translated / dual) plus, when a translation mode
//! is selected, a "Translate to" language list. Live status flows in via
//! `sourceNote` (transcription) and `translatedNote` (translation).

import { useEffect, useMemo, useRef, useState } from "react";
import { Captions, Search } from "lucide-react";

import { cn } from "@/lib/utils";
import type { CaptionMode } from "./CaptionOverlay";

export interface TargetLanguage {
  label: string;
  code: string;
}

interface CaptionPickerProps {
  mode: CaptionMode;
  onChange: (next: CaptionMode) => void;
  open: boolean;
  onClose: () => void;
  /// Live status for the "Source language" row, or null.
  sourceNote?: string | null;
  /// Live status for the "Translated" / "Dual" rows, or null.
  translatedNote?: string | null;
  /// Currently-selected reading language (BCP-47).
  targetLang: string;
  onTargetChange: (code: string) => void;
  /// Languages offered (from `list_target_languages`).
  languages: TargetLanguage[];
}

interface ModeOption {
  value: CaptionMode;
  label: string;
}

const OPTIONS: ModeOption[] = [
  { value: "off", label: "Off" },
  { value: "source", label: "Source language" },
  { value: "translation", label: "Translated" },
  { value: "dual", label: "Dual (source + translated)" },
];

export function CaptionPicker({
  mode,
  onChange,
  open,
  onClose,
  sourceNote,
  translatedNote,
  targetLang,
  onTargetChange,
  languages,
}: CaptionPickerProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [filter, setFilter] = useState("");

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [open, onClose]);

  const showTargets = mode === "translation" || mode === "dual";
  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return languages;
    return languages.filter(
      (l) => l.label.toLowerCase().includes(q) || l.code.toLowerCase().includes(q)
    );
  }, [filter, languages]);

  if (!open) return null;

  return (
    <div
      ref={ref}
      role="menu"
      className={cn(
        "nova-glass absolute bottom-12 right-0 z-30 w-64 overflow-hidden rounded-lg",
        "text-sm text-white"
      )}
    >
      <p className="px-3 py-2 text-[11px] uppercase tracking-wider text-white/55">Captions</p>
      <ul className="pb-1">
        {OPTIONS.map((opt) => {
          const isActive = mode === opt.value;
          const liveNote =
            opt.value === "source"
              ? sourceNote
              : opt.value === "translation" || opt.value === "dual"
                ? translatedNote
                : null;
          return (
            <li key={opt.value}>
              <button
                role="menuitemradio"
                aria-checked={isActive}
                onClick={() => {
                  onChange(opt.value);
                  // Keep the popover open for translation modes so the user can
                  // pick a language; close for off/source.
                  if (opt.value === "off" || opt.value === "source") onClose();
                }}
                className={cn(
                  "flex w-full items-center justify-between gap-3 px-3 py-2 text-left transition-colors",
                  "hover:bg-white/10",
                  isActive && "bg-white/10 text-white"
                )}
              >
                <span>{opt.label}</span>
                {liveNote ? (
                  <span className="text-[11px] text-[var(--color-accent)]">{liveNote}</span>
                ) : isActive ? (
                  <span className="text-[11px] text-[var(--color-accent)]">Selected</span>
                ) : null}
              </button>
            </li>
          );
        })}
      </ul>

      {showTargets && (
        <div className="border-t border-white/10">
          <p className="px-3 pb-1 pt-2 text-[11px] uppercase tracking-wider text-white/55">
            Translate to
          </p>
          <div className="relative px-2 pb-2">
            <Search className="pointer-events-none absolute left-4 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-white/40" />
            <input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Search language"
              className="w-full rounded-md border border-white/10 bg-white/[0.04] py-1.5 pl-8 pr-2 text-xs text-white placeholder:text-white/40 outline-none focus:border-white/25"
            />
          </div>
          <ul className="max-h-48 overflow-y-auto pb-1 [scrollbar-width:thin]">
            {filtered.map((l) => {
              const isSel = l.code === targetLang;
              return (
                <li key={l.code}>
                  <button
                    role="menuitemradio"
                    aria-checked={isSel}
                    onClick={() => onTargetChange(l.code)}
                    className={cn(
                      "flex w-full items-center justify-between gap-3 px-3 py-1.5 text-left transition-colors",
                      "hover:bg-white/10",
                      isSel && "bg-white/10"
                    )}
                  >
                    <span>{l.label}</span>
                    {isSel && (
                      <span className="text-[11px] text-[var(--color-accent)]">Selected</span>
                    )}
                  </button>
                </li>
              );
            })}
            {filtered.length === 0 && (
              <li className="px-3 py-2 text-[11px] text-white/40">No match</li>
            )}
          </ul>
        </div>
      )}
    </div>
  );
}

/// Toolbar button + integrated popover.
export function CaptionButton({
  mode,
  onChange,
  open,
  onToggle,
  onClose,
  sourceNote,
  translatedNote,
  targetLang,
  onTargetChange,
  languages,
}: {
  mode: CaptionMode;
  onChange: (m: CaptionMode) => void;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  sourceNote?: string | null;
  translatedNote?: string | null;
  targetLang: string;
  onTargetChange: (code: string) => void;
  languages: TargetLanguage[];
}) {
  const active = mode !== "off";
  return (
    <div className="relative">
      <button
        onClick={onToggle}
        aria-label="Captions"
        title="Captions"
        className={cn(
          "flex h-9 w-9 items-center justify-center rounded-full text-white/85",
          "transition-all hover:bg-white/15 hover:text-white hover:scale-105 active:scale-95",
          active && "bg-white/15 text-white"
        )}
      >
        <Captions className="h-5 w-5" />
      </button>
      <CaptionPicker
        mode={mode}
        onChange={onChange}
        open={open}
        onClose={onClose}
        sourceNote={sourceNote}
        translatedNote={translatedNote}
        targetLang={targetLang}
        onTargetChange={onTargetChange}
        languages={languages}
      />
    </div>
  );
}
