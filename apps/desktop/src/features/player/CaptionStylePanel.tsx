//! Caption-style controls — the Netflix-like "Aa" panel behind its own toolbar
//! button (Phase 6.2). Four control groups (size, font, colour, backdrop) plus a
//! live preview. Changes apply instantly through `useCaptionStyle`, which
//! persists them per profile.

import { useEffect, useRef } from "react";
import { Type } from "lucide-react";

import { cn } from "@/lib/utils";
import type {
  CaptionBackground,
  CaptionFontFamily,
  CaptionStyle,
  CaptionTextColor,
} from "@/types";
import { captionTextClass, captionTextStyle } from "./captionStyleCss";

interface PanelProps {
  style: CaptionStyle;
  onChange: (patch: Partial<CaptionStyle>) => void;
  open: boolean;
  onClose: () => void;
}

const SIZES: { label: string; value: number }[] = [
  { label: "S", value: 0.85 },
  { label: "M", value: 1.0 },
  { label: "L", value: 1.25 },
  { label: "XL", value: 1.5 },
];

const FONTS: { label: string; value: CaptionFontFamily }[] = [
  { label: "Sans", value: "sans" },
  { label: "Serif", value: "serif" },
  { label: "Mono", value: "mono" },
  { label: "Reader", value: "dyslexic" },
];

const COLORS: { value: CaptionTextColor; swatch: string }[] = [
  { value: "white", swatch: "#ffffff" },
  { value: "yellow", swatch: "#f5d90a" },
  { value: "cyan", swatch: "#5fe3e0" },
];

const BACKGROUNDS: { label: string; value: CaptionBackground }[] = [
  { label: "Shadow", value: "shadow" },
  { label: "Outline", value: "outline" },
  { label: "Box", value: "box" },
];

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="px-3 py-2">
      <p className="mb-1.5 text-[11px] uppercase tracking-wider text-white/55">{label}</p>
      <div className="flex items-center gap-1.5">{children}</div>
    </div>
  );
}

function Segment({
  active,
  onClick,
  children,
  ariaLabel,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
  ariaLabel?: string;
}) {
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "flex h-8 min-w-8 flex-1 items-center justify-center rounded-md px-2 text-xs",
        "transition-colors",
        active ? "bg-[var(--color-accent)] text-white" : "bg-white/10 text-white/80 hover:bg-white/20"
      )}
    >
      {children}
    </button>
  );
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
        "absolute bottom-12 right-0 z-30 w-72 overflow-hidden rounded-lg",
        "bg-black/85 backdrop-blur-md border border-white/10 shadow-xl text-white"
      )}
    >
      <p className="px-3 py-2 text-[11px] uppercase tracking-wider text-white/55">Caption style</p>

      {/* Live preview against a representative dark frame. */}
      <div className="mx-3 mb-1 flex h-14 items-center justify-center rounded-md bg-neutral-800/80">
        <span className={cn("font-medium leading-snug", captionTextClass(style))} style={captionTextStyle(style)}>
          Aa
        </span>
      </div>

      <Row label="Text size">
        {SIZES.map((s) => (
          <Segment
            key={s.label}
            active={Math.abs(style.fontScale - s.value) < 1e-3}
            onClick={() => onChange({ fontScale: s.value })}
            ariaLabel={`Text size ${s.label}`}
          >
            {s.label}
          </Segment>
        ))}
      </Row>

      <Row label="Font">
        {FONTS.map((f) => (
          <Segment key={f.value} active={style.fontFamily === f.value} onClick={() => onChange({ fontFamily: f.value })}>
            {f.label}
          </Segment>
        ))}
      </Row>

      <Row label="Colour">
        {COLORS.map((c) => (
          <button
            key={c.value}
            type="button"
            aria-label={`Text colour ${c.value}`}
            aria-pressed={style.textColor === c.value}
            onClick={() => onChange({ textColor: c.value })}
            className={cn(
              "h-8 w-8 rounded-full border-2 transition-transform hover:scale-105",
              style.textColor === c.value ? "border-[var(--color-accent)]" : "border-white/25"
            )}
            style={{ backgroundColor: c.swatch }}
          />
        ))}
      </Row>

      <Row label="Background">
        {BACKGROUNDS.map((b) => (
          <Segment key={b.value} active={style.background === b.value} onClick={() => onChange({ background: b.value })}>
            {b.label}
          </Segment>
        ))}
      </Row>

      {style.background === "box" && (
        <div className="px-3 pb-3 pt-1">
          <p className="mb-1.5 text-[11px] uppercase tracking-wider text-white/55">Box opacity</p>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={style.bgOpacity}
            onChange={(e) => onChange({ bgOpacity: Number(e.target.value) })}
            className="w-full accent-[var(--color-accent)]"
            aria-label="Box opacity"
          />
        </div>
      )}
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
