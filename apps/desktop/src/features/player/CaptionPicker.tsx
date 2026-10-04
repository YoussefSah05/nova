//! Captions menu — the popover behind the CC button.
//!
//! Display mode (off / source). Live transcription status flows in via
//! `sourceNote`.

import { useEffect, useRef } from "react";
import { Captions } from "lucide-react";

import { cn } from "@/lib/utils";
import type { CaptionMode } from "./CaptionOverlay";

interface CaptionPickerProps {
  mode: CaptionMode;
  onChange: (next: CaptionMode) => void;
  open: boolean;
  onClose: () => void;
  /// Live status for the "Source language" row, or null.
  sourceNote?: string | null;
}

interface ModeOption {
  value: CaptionMode;
  label: string;
}

const OPTIONS: ModeOption[] = [
  { value: "off", label: "Off" },
  { value: "source", label: "Source language" },
];

export function CaptionPicker({ mode, onChange, open, onClose, sourceNote }: CaptionPickerProps) {
  const ref = useRef<HTMLDivElement | null>(null);

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
        "nova-glass nova-panel-enter absolute bottom-12 right-0 z-30 w-64 overflow-hidden rounded-xl",
        "text-sm text-white"
      )}
    >
      <p className="px-3 py-2 text-[11px] uppercase tracking-wider text-white/55">Captions</p>
      <ul className="pb-1">
        {OPTIONS.map((opt) => {
          const isActive = mode === opt.value;
          const liveNote = opt.value === "source" ? sourceNote : null;
          return (
            <li key={opt.value}>
              <button
                role="menuitemradio"
                aria-checked={isActive}
                onClick={() => {
                  onChange(opt.value);
                  onClose();
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
      <div className="border-t border-white/10 px-3 py-3">
        <p className="text-xs text-primary">Translation & dual captions</p>
        <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
          Coming with the intelligence upgrade. Source captions are available today.
        </p>
      </div>
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
}: {
  mode: CaptionMode;
  onChange: (m: CaptionMode) => void;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
  sourceNote?: string | null;
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
      />
    </div>
  );
}
