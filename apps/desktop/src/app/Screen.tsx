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
import { ScreenActiveContext } from "@/lib/screenActive";
import { ScreenScrollContext } from "./screenContext";

/// Quiet time after the last scroll event before the pointer is live again.
const SCROLL_IDLE_MS = 150;
/// Pointer travel during a scroll that counts as intent to use the cards.
const RELEASE_PX = 6;

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
  /// Kept current by a passive scroll listener, so it is never read after
  /// the screen was hidden (when an engine might report 0).
  const savedTop = useRef(0);
  /// What had focus when the screen was hidden, to give it back on return.
  const lastFocus = useRef<HTMLElement | null>(null);
  const scrolled = useRef(false);
  const report = useRef(onScrolledChange);
  report.current = onScrolledChange;
  const activeRef = useRef(active);
  activeRef.current = active;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    // While the screen or a horizontal rail scrolls, cards skip hover decoration:
    // cards passing under a resting cursor would otherwise run hover styles (large
    // shadow repaints in WebKit) and start hover previews and detail
    // prefetches mid-scroll. A class on the container, never a React render;
    // two style passes per gesture. Activation stays clickable during momentum.
    let idle = 0;
    const onScroll = () => {
      if (!activeRef.current) return;
      savedTop.current = el.scrollTop;
      if (!idle) {
        el.classList.add("nova-scrolling");
        anchor = pointer;
      }
      window.clearTimeout(idle);
      idle = window.setTimeout(() => {
        idle = 0;
        anchor = null;
        el.classList.remove("nova-scrolling");
      }, SCROLL_IDLE_MS);
    };
    // Moving the pointer a few pixels hands the cards back at once (e.g. to
    // click one while momentum settles). Positions, not `movementX`, which
    // some WebKit builds report as 0; the tolerance ignores hand jitter
    // during wheel scrolling.
    let pointer: { x: number; y: number } | null = null;
    let anchor: { x: number; y: number } | null = null;
    const onPointerMove = (event: PointerEvent) => {
      if (!Number.isFinite(event.clientX) || !Number.isFinite(event.clientY)) return;
      pointer = { x: event.clientX, y: event.clientY };
      if (!idle) return;
      if (!anchor) {
        anchor = pointer;
        return;
      }
      if (Math.hypot(pointer.x - anchor.x, pointer.y - anchor.y) < RELEASE_PX) return;
      window.clearTimeout(idle);
      idle = 0;
      anchor = null;
      el.classList.remove("nova-scrolling");
    };
    el.addEventListener("scroll", onScroll, { passive: true, capture: true });
    el.addEventListener("pointermove", onPointerMove, { passive: true });
    return () => {
      window.clearTimeout(idle);
      el.removeEventListener("scroll", onScroll, true);
      el.removeEventListener("pointermove", onPointerMove);
    };
  }, []);

  // Scrolled past the top ↔ the sentinel left the viewport. One observer
  // callback per crossing, never per scroll frame; ignored while hidden.
  useEffect(() => {
    const root = ref.current;
    const sentinel = sentinelRef.current;
    if (!root || !sentinel || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!activeRef.current) return;
        scrolled.current = !entry.isIntersecting;
        report.current(scrolled.current);
      },
      { root }
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, []);

  // On show: the same scroll offset, the top bar's state, and focus back
  // where it was (or on the screen) if it was left on a hidden screen.
  // On hide: remember what had focus here.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (!active) {
      const focused = document.activeElement;
      if (focused instanceof HTMLElement && el.contains(focused)) lastFocus.current = focused;
      return;
    }
    if (el.scrollTop !== savedTop.current) el.scrollTop = savedTop.current;
    report.current(scrolled.current);
    const focused = document.activeElement;
    const stranded =
      !focused ||
      focused === document.body ||
      (focused instanceof HTMLElement && !!focused.closest("[inert]"));
    if (stranded) {
      const target = lastFocus.current?.isConnected ? lastFocus.current : el;
      target.focus({ preventScroll: true });
    }
  }, [active]);

  return (
    <div
      ref={ref}
      role="region"
      aria-label={label}
      aria-hidden={!active}
      inert={!active}
      tabIndex={-1}
      className={cn(
        "nova-screen absolute inset-0 overflow-y-auto overflow-x-hidden outline-none",
        active ? "nova-screen-enter" : "nova-screen-hidden"
      )}
    >
      <div
        ref={sentinelRef}
        aria-hidden
        className="pointer-events-none absolute left-0 top-0 h-6 w-px"
      />
      <ScreenActiveContext.Provider value={active}>
        <ScreenScrollContext.Provider value={ref}>{children}</ScreenScrollContext.Provider>
      </ScreenActiveContext.Provider>
    </div>
  );
});
