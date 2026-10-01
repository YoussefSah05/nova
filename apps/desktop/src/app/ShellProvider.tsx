//! Overlay state (`shell.ts`) plus the Settings and Add-playlist modals.
//! The command palette is rendered by the shell, which knows navigation.

import { useEffect, useMemo, useState, type ReactNode } from "react";

import { AddSourceModal } from "@/features/source/AddSourceModal";
import { SettingsModal } from "@/features/settings/SettingsModal";
import { usePlaying } from "./playback";
import { useSession } from "./session";
import { ShellContext, type Shell } from "./shell";

export function ShellProvider({ children }: { children: ReactNode }) {
  const session = useSession();
  const playing = usePlaying();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [addSourceOpen, setAddSourceOpen] = useState(false);

  // ⌘K anywhere (except over the player) toggles the palette.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (
        (e.metaKey || e.ctrlKey) &&
        e.key.toLowerCase() === "k" &&
        !playing &&
        !settingsOpen &&
        !addSourceOpen
      ) {
        e.preventDefault();
        setPaletteOpen((open) => !open);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [playing, settingsOpen, addSourceOpen]);

  const value = useMemo<Shell>(
    () => ({
      paletteOpen,
      setPaletteOpen,
      openSettings: () => setSettingsOpen(true),
      openAddSource: () => setAddSourceOpen(true),
      overlayOpen: paletteOpen || settingsOpen || addSourceOpen || playing,
    }),
    [paletteOpen, settingsOpen, addSourceOpen, playing]
  );

  return (
    <ShellContext.Provider value={value}>
      {children}
      {addSourceOpen && (
        <AddSourceModal
          profileId={session.profile.id}
          onDone={() => {
            setAddSourceOpen(false);
            void session.sourceAdded();
          }}
          onCancel={() => setAddSourceOpen(false)}
        />
      )}
      {settingsOpen && (
        <SettingsModal
          key={session.profile.id}
          profile={session.profile}
          onClose={() => setSettingsOpen(false)}
          onSwitchProfile={session.switchProfile}
          onProfileUpdated={session.setProfile}
          onSourcesChanged={session.reloadSources}
          onAddSource={() => {
            setSettingsOpen(false);
            setAddSourceOpen(true);
          }}
        />
      )}
    </ShellContext.Provider>
  );
}
