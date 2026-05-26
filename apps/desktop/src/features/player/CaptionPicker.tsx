//! Captions menu - the popover behind the CC button.
//!
//! Phase 5 surface is intentionally thin: "Off" is the only enabled mode,
//! and the other options are visible-but-disabled so users see what's coming.
//! Phase 6 enables "Source", Phase 7 enables translation and dual modes.

import { useEffect, useRef } from "react";
import { Captions } from "lucide-react";

import { cn } from "@/lib/utils";
import type { CaptionMode } from "./CaptionOverlay";

interface CaptionPickerProps {
  mode: CaptionMode;
  onChange: (next: CaptionMode) => void;
  /// When `true` the popover is visible.
  open: boolean;
  onClose: () => void;
}

interface ModeOption {
  value: CaptionMode;
  label: string;
  /// `null` when enabled in Phase 5. When set, shows the gate-phase note.
  disabledNote: string | null;
}

const OPTIONS: ModeOption[] = [
  { value: "off", label: "Off", disabledNote: null },
  { value: "source", label: "Source language", disabledNote: "Coming in Phase 6" },
  { value: "translation", label: "Translated", disabledNote: "Coming in Phase 7" },
  { value: "dual", label: "Dual (source + translated)", disabledNote: "Coming in Phase 7" },
];

export function CaptionPicker({ mode, onChange, open, onClose }: CaptionPickerProps) {
  const ref = useRef<HTMLDivElement | null>(null);

  // Click-outside-to-close. Cheap and matches the rest of the player popovers.
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      ref={ref}
      role="menu"
      className={cn(
        "absolute bottom-12 right-0 z-30 w-64 overflow-hidden rounded-lg",
        "bg-black/85 backdrop-blur-md border border-white/10 shadow-xl",
        "text-sm text-white"
      )}
    >
      <p className="px-3 py-2 text-[11px] uppercase tracking-wider text-white/55">Captions</p>
      <ul className="pb-1">
        {OPTIONS.map((opt) => {
          const isActive = mode === opt.value;
          const isDisabled = opt.disabledNote !== null;
          return (
            <li key={opt.value}>
              <button
                role="menuitemradio"
                aria-checked={isActive}
                disabled={isDisabled}
                onClick={() => {
                  if (isDisabled) return;
                  onChange(opt.value);
                  onClose();
                }}
                className={cn(
                  "flex w-full items-center justify-between gap-3 px-3 py-2 text-left",
                  "transition-colors",
                  isDisabled ? "cursor-not-allowed text-white/40" : "hover:bg-white/10",
                  isActive && "bg-white/10 text-white"
                )}
              >
                <span>{opt.label}</span>
                {isDisabled ? (
                  <span className="text-[11px] text-white/40">{opt.disabledNote}</span>
                ) : isActive ? (
                  <span className="text-[11px] text-[var(--color-accent)]">Selected</span>
                ) : null}
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

/// Toolbar button + integrated popover. The player drops this in alongside
/// the existing fullscreen / PiP icons.
export function CaptionButton({
  mode,
  onChange,
  open,
  onToggle,
  onClose,
}: {
  mode: CaptionMode;
  onChange: (m: CaptionMode) => void;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
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
      <CaptionPicker mode={mode} onChange={onChange} open={open} onClose={onClose} />
    </div>
  );
}
