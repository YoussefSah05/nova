//! One screen of the shell (a section root or a pushed page) in its own
//! scroll container. Screens stay mounted while their section keeps them, so
//! going back or switching sections shows them exactly as left: same scroll
//! position, same images, no refetch, no re-layout.
//!
//! Inactive screens are hidden with `visibility` (layout and scroll offsets
//! kept) plus `content-visibility: hidden` where WebKit has it, which also
//! skips their painting and hit-testing. The scroll offset is still saved and
//! re-applied on activation, in case an engine resets it.

import { memo, useEffect, useLayoutEffect, useRef, type ReactNode } from "react";

import { cn } from "@/lib/utils";
import { ScreenScrollContext } from "./screenContext";

export const Screen = memo(function Screen({
  active,
  label,
  onScrolledChange,
  children,
}: {
  active: boolean;
  label: string;
  /// Whether the screen is scrolled past its top (the top bar turns to glass).
  onScrolledChange: (scrolled: boolean) => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const savedTop = useRef(0);
  const scrolled = useRef(false);
  const report = useRef(onScrolledChange);
  report.current = onScrolledChange;
  const activeRef = useRef(active);
  activeRef.current = active;

  // Scrolled past the top ↔ the sentinel left the viewport. One observer
  // callback per crossing, never per scroll frame.
  useEffect(() => {
    const root = ref.current;
    const sentinel = sentinelRef.current;
    if (!root || !sentinel || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        scrolled.current = !entry.isIntersecting;
        if (activeRef.current) report.current(scrolled.current);
      },
      { root }
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, []);

  // Keep the scroll offset across hiding; tell the top bar this screen's state.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (active) {
      if (el.scrollTop !== savedTop.current) el.scrollTop = savedTop.current;
      report.current(scrolled.current);
    } else {
      savedTop.current = el.scrollTop;
    }
  }, [active]);

  return (
    <div
      ref={ref}
      role="region"
      aria-label={label}
      aria-hidden={!active}
      inert={!active}
      className={cn(
        "nova-screen absolute inset-0 overflow-y-auto overflow-x-hidden",
        !active && "nova-screen-hidden"
      )}
    >
      <div
        ref={sentinelRef}
        aria-hidden
        className="pointer-events-none absolute left-0 top-0 h-6 w-px"
      />
      <ScreenScrollContext.Provider value={ref}>{children}</ScreenScrollContext.Provider>
    </div>
  );
});
