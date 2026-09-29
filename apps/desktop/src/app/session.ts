//! The signed-in session: profile, playlists and the active playlist
//! (stable; read by every screen), and its status — playlist totals, refresh
//! state, the error line — in a separate context that only the chrome reads,
//! so a status change never re-renders the screens kept mounted underneath.
//! Provided by `SessionProvider.tsx`.

import { createContext, useContext } from "react";

import type { Profile, Source } from "@/types";

export interface Session {
  profile: Profile;
  /// Show a rename/recolour from Settings immediately.
  setProfile: (profile: Profile) => void;
  switchProfile: () => void;
  sources: Source[];
  /// Loaded at least once (distinguishes "no playlists" from "loading").
  sourcesLoaded: boolean;
  activeSourceId: number | null;
  setActiveSourceId: (id: number) => void;
  setError: (message: string | null) => void;
  /// Re-fetch the active playlist from the provider.
  refresh: () => Promise<void>;
  /// Re-read the playlists (after Settings added or removed one).
  reloadSources: () => void;
  /// A playlist was just added: select it.
  sourceAdded: () => Promise<void>;
}

export interface SessionStatus {
  /// Items per playlist, for the switcher's labels.
  sourceTotals: Record<number, number>;
  error: string | null;
  refreshing: boolean;
}

export const SessionContext = createContext<Session | null>(null);
export const SessionStatusContext = createContext<SessionStatus>({
  sourceTotals: {},
  error: null,
  refreshing: false,
});

export function useSessionStatus(): SessionStatus {
  return useContext(SessionStatusContext);
}

export function useSession(): Session {
  const session = useContext(SessionContext);
  if (!session) throw new Error("useSession outside SessionProvider");
  return session;
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (typeof error === "object" && error !== null) {
    const maybeMessage = "message" in error ? error.message : null;
    if (typeof maybeMessage === "string") return maybeMessage;
  }
  return "Something went wrong while loading imported content.";
}
