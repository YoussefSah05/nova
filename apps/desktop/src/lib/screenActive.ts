//! Whether the component renders on the screen currently shown. Screens stay
//! mounted while hidden (see `app/Screen.tsx`); their queries pass
//! `subscribed: useScreenActive()`, so a hidden screen neither refetches nor
//! re-renders when its data is invalidated — it catches up when shown again.
//! Outside any screen (player, palette, settings) it is always true.

import { createContext, useContext } from "react";

export const ScreenActiveContext = createContext(true);

export function useScreenActive(): boolean {
  return useContext(ScreenActiveContext);
}
