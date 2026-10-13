//! In-app confirmation for destructive Settings actions.
//!
//! `window.confirm` cannot be used: the macOS WebView has no dialog for it and
//! returns `false` without showing anything, so every guarded action silently
//! did nothing. `useConfirm()` asks through Nova's own dialog instead and
//! resolves to the user's answer.

import { createContext, useContext } from "react";

export interface ConfirmRequest {
  title: string;
  description: string;
  /** Label of the button that goes ahead, e.g. "Remove playlist". */
  confirmLabel: string;
  /** Red confirm button for actions that delete something. */
  destructive?: boolean;
}

export type Confirm = (request: ConfirmRequest) => Promise<boolean>;

export const ConfirmContext = createContext<Confirm | null>(null);

/** Ask the user to confirm; resolves `true` only if they choose to go ahead. */
export function useConfirm(): Confirm {
  const confirm = useContext(ConfirmContext);
  if (!confirm) {
    throw new Error("useConfirm needs a <ConfirmProvider> above it");
  }
  return confirm;
}
