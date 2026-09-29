//! The navigation contexts and their hooks (the provider is
//! `NavProvider.tsx`). Actions and state are separate contexts: the actions
//! never change, so cards and pages that only navigate don't re-render when
//! another screen is pushed — which matters because screens stay mounted.

import { createContext, useContext } from "react";

import type { Entry, NavAction, NavState, Route, SectionId } from "./nav";

export interface NavActions {
  push: (route: Route) => void;
  back: () => void;
  switchTo: (section: SectionId) => void;
  patchItem: (key: string, patch: Extract<NavAction, { type: "patchItem" }>["patch"]) => void;
}

export interface NavApi extends NavActions {
  state: NavState;
  current: Entry;
  canGoBack: boolean;
}

export const NavContext = createContext<NavApi | null>(null);
export const NavActionsContext = createContext<NavActions | null>(null);

/// Navigation state and actions (re-renders on every navigation).
export function useNav(): NavApi {
  const nav = useContext(NavContext);
  if (!nav) throw new Error("useNav outside NavProvider");
  return nav;
}

/// Navigation actions only (stable; never re-renders).
export function useNavActions(): NavActions {
  const actions = useContext(NavActionsContext);
  if (!actions) throw new Error("useNavActions outside NavProvider");
  return actions;
}
