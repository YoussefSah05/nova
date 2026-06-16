//! Caption-style controls — the Netflix-like "Aa" panel behind its own toolbar
//! button (Phase 6.2). Four control groups (size, font, colour, backdrop) plus a
//! live preview. Changes apply instantly through `useCaptionStyle`, which
//! persists them per profile.

import { useEffect, useRef } from "react";
import { Type } from "lucide-react";

import { cn } from "@/lib/utils";
import type { CaptionStyle } from "@/types";
import { CaptionStyleControls } from "./CaptionStyleControls";

interface PanelProps {
  style: CaptionStyle;
  onChange: (patch: Partial<CaptionStyle>) => void;
  open: boolean;
  onClose: () => void;
}

export function CaptionStylePanel({ style, onChange, open, onClose }: PanelProps) {
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
        "nova-glass absolute bottom-12 right-0 z-30 w-72 overflow-hidden rounded-lg pb-1 text-white"
      )}
    >
      <p className="px-3 py-2 text-[11px] uppercase tracking-wider text-white/55">Caption style</p>
      <CaptionStyleControls style={style} onChange={onChange} />
    </div>
  );
}

/// Toolbar button + integrated panel. Dropped in next to the captions button.
export function CaptionStyleButton({
  style,
  onChange,
  open,
  onToggle,
  onClose,
}: {
  style: CaptionStyle;
  onChange: (patch: Partial<CaptionStyle>) => void;
  open: boolean;
  onToggle: () => void;
  onClose: () => void;
}) {
  return (
    <div className="relative">
      <button
        onClick={onToggle}
        aria-label="Caption style"
        title="Caption style"
        className={cn(
          "flex h-9 w-9 items-center justify-center rounded-full text-white/85",
          "transition-all hover:bg-white/15 hover:text-white hover:scale-105 active:scale-95",
          open && "bg-white/15 text-white"
        )}
      >
        <Type className="h-5 w-5" />
      </button>
      <CaptionStylePanel style={style} onChange={onChange} open={open} onClose={onClose} />
    </div>
  );
}
