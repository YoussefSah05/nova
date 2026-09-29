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
    const onScroll = () => {
      if (activeRef.current) savedTop.current = el.scrollTop;
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
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
        !active && "nova-screen-hidden"
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
