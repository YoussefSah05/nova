//! Reusable caption-style controls (preview + size/font/colour/backdrop), shared
//! by the in-player "Aa" panel (`CaptionStylePanel`) and the Settings →
//! Playback & Captions section so the two can't drift.

import { cn } from "@/lib/utils";
import type { CaptionBackground, CaptionFontFamily, CaptionStyle, CaptionTextColor } from "@/types";
import { captionTextClass, captionTextStyle } from "./captionStyleCss";

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
        "flex h-8 min-w-8 flex-1 items-center justify-center rounded-md px-2 text-xs transition-colors",
        active
          ? "bg-primary font-medium text-primary-foreground"
          : "bg-white/10 text-white/80 hover:bg-white/20"
      )}
    >
      {children}
    </button>
  );
}

export function CaptionStyleControls({
  style,
  onChange,
  preview = true,
}: {
  style: CaptionStyle;
  onChange: (patch: Partial<CaptionStyle>) => void;
  preview?: boolean;
}) {
  return (
    <div>
      {preview && (
        <div className="mx-3 mb-1 flex h-14 items-center justify-center rounded-md bg-neutral-800/80">
          <span
            className={cn("font-medium leading-snug", captionTextClass(style))}
            style={captionTextStyle(style)}
          >
            Aa
          </span>
        </div>
      )}

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
          <Segment
            key={f.value}
            active={style.fontFamily === f.value}
            onClick={() => onChange({ fontFamily: f.value })}
          >
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
              style.textColor === c.value ? "border-primary" : "border-white/25"
            )}
            style={{ backgroundColor: c.swatch }}
          />
        ))}
      </Row>

      <Row label="Background">
        {BACKGROUNDS.map((b) => (
          <Segment
            key={b.value}
            active={style.background === b.value}
            onClick={() => onChange({ background: b.value })}
          >
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
            className="w-full accent-[var(--color-primary)]"
            aria-label="Box opacity"
          />
        </div>
      )}
    </div>
  );
}
