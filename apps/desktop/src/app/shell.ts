//! The shell's overlays (command palette, settings, add playlist). Provided
//! by `ShellProvider.tsx`; navigation shortcuts pause while one is open.

import { createContext, useContext } from "react";

export interface Shell {
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  openSettings: () => void;
  openAddSource: () => void;
  /// An overlay or the player owns the keyboard.
  overlayOpen: boolean;
}

export const ShellContext = createContext<Shell | null>(null);

export function useShell(): Shell {
  const shell = useContext(ShellContext);
  if (!shell) throw new Error("useShell outside ShellProvider");
  return shell;
}
