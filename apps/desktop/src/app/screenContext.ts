//! The scroll container of the screen a component renders in (virtualized
//! lists window against it). Provided by `Screen.tsx`.

import { createContext, useContext, type RefObject } from "react";

export const ScreenScrollContext = createContext<RefObject<HTMLElement | null> | null>(null);

export function useScreenScroll(): RefObject<HTMLElement | null> {
  const ref = useContext(ScreenScrollContext);
  if (!ref) throw new Error("useScreenScroll outside a Screen");
  return ref;
}
