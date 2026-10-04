import { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Pause, Play } from "lucide-react";
import type { ContinueWatchingItem } from "@/types";
import { Button } from "@/components/ui/button";
import { useShell } from "@/app/shell";
import { useScreenActive } from "@/lib/screenActive";
import { useReducedMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";
import { ResumeHero } from "./ResumeHero";
import type { ShowcaseSlide } from "./showcase";

export const SHOWCASE_INTERVAL = 9000;

export function HomeShowcase({
  slides,
  alsoInProgress,
}: {
  slides: ShowcaseSlide[];
  alsoInProgress: ContinueWatchingItem[];
}) {
  const active = useScreenActive();
  const { overlayOpen } = useShell();
  const reduce = useReducedMotion();
  const ref = useRef<HTMLDivElement>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [paused, setPaused] = useState(false);
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [visible, setVisible] = useState(false);
  const [documentVisible, setDocumentVisible] = useState(!document.hidden);
  const index = Math.max(
    0,
    slides.findIndex((slide) => slide.key === selected)
  );
  const slide = slides[index];
  const rotating =
    slides.length > 1 &&
    active &&
    !overlayOpen &&
    visible &&
    documentVisible &&
    !reduce &&
    !paused &&
    !hovered &&
    !focused;

  useEffect(() => {
    const changed = () => setDocumentVisible(!document.hidden);
    document.addEventListener("visibilitychange", changed);
    return () => document.removeEventListener("visibilitychange", changed);
  }, []);
  useEffect(() => {
    if (!ref.current || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(([entry]) => setVisible(entry.isIntersecting), {
      threshold: 0.25,
    });
    observer.observe(ref.current);
    return () => observer.disconnect();
  }, []);
  const keys = slides.map((item) => item.key).join("|");
  useEffect(() => {
    if (!rotating) return;
    const timer = window.setTimeout(
      () => setSelected(slides[(index + 1) % slides.length].key),
      SHOWCASE_INTERVAL
    );
    return () => window.clearTimeout(timer);
  }, [rotating, index, keys, slides]);

  function select(next: number) {
    setPaused(true);
    setSelected(slides[(next + slides.length) % slides.length].key);
  }

  return (
    <div
      ref={ref}
      role="region"
      aria-roledescription="carousel"
      aria-label="Tonight’s showcase"
      className="relative"
      onPointerEnter={() => setHovered(true)}
      onPointerLeave={() => setHovered(false)}
      onFocusCapture={() => setFocused(true)}
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false);
      }}
    >
      <div
        key={slide?.key ?? "empty"}
        className="nova-showcase-enter"
        role="group"
        aria-roledescription="slide"
        aria-label={slide ? `${index + 1} of ${slides.length}: ${slide.title}` : "Welcome"}
      >
        <ResumeHero
          item={slide?.kind === "resume" ? slide.item : null}
          spotlight={slide?.kind === "title" ? slide.item : null}
          alsoInProgress={alsoInProgress}
        />
      </div>
      {slides.length > 1 && (
        <div
          className="absolute bottom-6 left-6 right-6 flex items-center justify-between gap-4 md:left-[88px] md:right-14"
          aria-label="Showcase controls"
        >
          <div className="flex items-center gap-1">
            {slides.map((item, i) => (
              <button
                key={item.key}
                type="button"
                aria-label={`Show ${item.title}`}
                aria-pressed={index === i}
                onClick={() => select(i)}
                className="flex h-8 w-8 items-center justify-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span
                  className={cn(
                    "h-1 w-6 rounded-full transition-[opacity,transform] duration-200",
                    index === i
                      ? "scale-x-100 bg-primary opacity-100"
                      : "scale-x-50 bg-white opacity-40"
                  )}
                />
              </button>
            ))}
            <span className="ml-3 text-xs tabular-nums text-white/60">
              {index + 1} / {slides.length}
            </span>
          </div>
          <div className="flex gap-1">
            {!reduce && (
              <Button
                variant="ghost"
                size="iconSm"
                aria-label={paused ? "Start showcase rotation" : "Pause showcase rotation"}
                onClick={() => setPaused((value) => !value)}
              >
                {paused ? <Play /> : <Pause />}
              </Button>
            )}
            <Button
              variant="ghost"
              size="iconSm"
              aria-label="Previous showcase title"
              onClick={() => select(index - 1)}
            >
              <ChevronLeft />
            </Button>
            <Button
              variant="ghost"
              size="iconSm"
              aria-label="Next showcase title"
              onClick={() => select(index + 1)}
            >
              <ChevronRight />
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
