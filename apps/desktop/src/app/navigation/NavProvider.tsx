//! Navigation context: the per-section stacks (`nav.ts`), the keyboard and
//! mouse shortcuts, and a cross-fade between screens where the webview has
//! View Transitions (WebKit 18+). Without them, or with reduced motion, the
//! swap is instant — screens stay mounted, so there is nothing to load.

import { useCallback, useEffect, useMemo, useReducer, useRef, type ReactNode } from "react";
import { flushSync } from "react-dom";

import {
  canGoBack,
  currentEntry,
  initialNav,
  navReducer,
  SECTIONS,
  type Route,
  type SectionId,
} from "./nav";
import { NavActionsContext, NavContext, type NavActions, type NavApi } from "./useNav";

type ViewTransitionDocument = Document & {
  startViewTransition?: (update: () => void) => unknown;
};

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/// Run a navigation update inside a view transition when supported.
function withTransition(update: () => void) {
  const doc = document as ViewTransitionDocument;
  if (typeof doc.startViewTransition !== "function" || prefersReducedMotion()) {
    update();
    return;
  }
  doc.startViewTransition(() => flushSync(update));
}

function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

export function NavProvider({
  children,
  blocked,
  initialSection = "home",
}: {
  children: ReactNode;
  /// True while an overlay (player, palette, modal) owns the keyboard.
  blocked: () => boolean;
  initialSection?: SectionId;
}) {
  const [state, dispatch] = useReducer(navReducer, initialSection, initialNav);
  const stateRef = useRef(state);
  stateRef.current = state;

  const push = useCallback(
    (route: Route) => withTransition(() => dispatch({ type: "push", route })),
    []
  );
  const back = useCallback(() => {
    if (canGoBack(stateRef.current)) withTransition(() => dispatch({ type: "pop" }));
  }, []);
  const switchTo = useCallback(
    (section: SectionId) => withTransition(() => dispatch({ type: "switch", section })),
    []
  );
  const patchItem = useCallback<NavApi["patchItem"]>(
    (key, patch) => dispatch({ type: "patchItem", key, patch }),
    []
  );

  const blockedRef = useRef(blocked);
  blockedRef.current = blocked;

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.defaultPrevented || blockedRef.current()) return;
      const mod = e.metaKey || e.ctrlKey;
      if (mod && /^[1-5]$/.test(e.key)) {
        e.preventDefault();
        switchTo(SECTIONS[Number(e.key) - 1]);
        return;
      }
      if ((mod && e.key === "[") || (e.key === "Escape" && !isEditable(e.target))) {
        if (!canGoBack(stateRef.current)) return;
        e.preventDefault();
        back();
      }
    }
    // The mouse's back button (button 3).
    function onMouse(e: MouseEvent) {
      if (e.button === 3 && !blockedRef.current()) {
        e.preventDefault();
        back();
      }
    }
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, [back, switchTo]);

  const actions = useMemo<NavActions>(
    () => ({ push, back, switchTo, patchItem }),
    [push, back, switchTo, patchItem]
  );

  const api = useMemo<NavApi>(
    () => ({
      state,
      current: currentEntry(state),
      canGoBack: canGoBack(state),
      push,
      back,
      switchTo,
      patchItem,
    }),
    [state, push, back, switchTo, patchItem]
  );

  return (
    <NavActionsContext.Provider value={actions}>
      <NavContext.Provider value={api}>{children}</NavContext.Provider>
    </NavActionsContext.Provider>
  );
}
