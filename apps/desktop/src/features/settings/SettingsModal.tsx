//! Settings overlay — left section sidebar + right content pane.

import { useState } from "react";
import { Captions, Cpu, Palette, Sparkles, Tv, UserCog, X } from "lucide-react";

import { cn } from "@/lib/utils";
import { motion } from "@/lib/motion";
import type { Profile } from "@/types";
import { PlaybackCaptionsSection } from "./sections/PlaybackCaptionsSection";
import { TranscriptionSection } from "./sections/TranscriptionSection";
import { EnrichmentSection } from "./sections/EnrichmentSection";
import { AppearanceSection } from "./sections/AppearanceSection";
import { SourcesEpgSection } from "./sections/SourcesEpgSection";
import { ProfileAboutSection } from "./sections/ProfileAboutSection";

type SectionId = "appearance" | "playback" | "transcription" | "enrichment" | "sources" | "profile";

const SECTIONS: { id: SectionId; label: string; icon: React.ReactNode }[] = [
  { id: "appearance", label: "Appearance", icon: <Palette className="h-4 w-4" /> },
  { id: "playback", label: "Playback & Captions", icon: <Captions className="h-4 w-4" /> },
  { id: "transcription", label: "Transcription & Cache", icon: <Cpu className="h-4 w-4" /> },
  { id: "enrichment", label: "Catalog Enrichment", icon: <Sparkles className="h-4 w-4" /> },
  { id: "sources", label: "Sources & EPG", icon: <Tv className="h-4 w-4" /> },
  { id: "profile", label: "Profile & About", icon: <UserCog className="h-4 w-4" /> },
];

export function SettingsModal({
  profile,
  onClose,
  onSwitchProfile,
  onProfileUpdated,
  onSourcesChanged,
}: {
  profile: Profile;
  onClose: () => void;
  onSwitchProfile: () => void;
  onProfileUpdated: (p: Profile) => void;
  onSourcesChanged: () => void;
}) {
  const [active, setActive] = useState<SectionId>("playback");

  return (
    <motion.div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-6"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.15 }}
    >
      <motion.div
        className="flex h-[80vh] w-full max-w-4xl overflow-hidden rounded-2xl border border-white/10 bg-neutral-950 shadow-2xl"
        initial={{ opacity: 0, scale: 0.98, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
      >
        {/* Sidebar */}
        <aside className="flex w-60 shrink-0 flex-col border-r border-white/10 bg-white/[0.02] p-3">
          <p className="px-2 py-2 text-sm font-semibold text-white">Settings</p>
          <nav className="mt-1 flex flex-col gap-1">
            {SECTIONS.map((s) => (
              <button
                key={s.id}
                type="button"
                onClick={() => setActive(s.id)}
                className={cn(
                  "flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition-colors",
                  active === s.id
                    ? "bg-white/10 text-white"
                    : "text-white/65 hover:bg-white/5 hover:text-white"
                )}
              >
                {s.icon}
                <span>{s.label}</span>
              </button>
            ))}
          </nav>
        </aside>

        {/* Content */}
        <div className="flex min-w-0 flex-1 flex-col">
          <header
            data-tauri-drag-region
            className="flex items-center justify-between border-b border-white/10 px-6 py-3"
          >
            <h2 className="text-sm font-semibold text-white">
              {SECTIONS.find((s) => s.id === active)?.label}
            </h2>
            <button
              type="button"
              onClick={onClose}
              aria-label="Close settings"
              className="flex h-8 w-8 items-center justify-center rounded-full text-white/70 transition-colors hover:bg-white/10 hover:text-white"
            >
              <X className="h-5 w-5" />
            </button>
          </header>
          <div className="flex-1 overflow-y-auto px-6 py-5">
            {active === "appearance" && <AppearanceSection profileId={profile.id} />}
            {active === "playback" && <PlaybackCaptionsSection profileId={profile.id} />}
            {active === "transcription" && <TranscriptionSection profileId={profile.id} />}
            {active === "enrichment" && <EnrichmentSection profileId={profile.id} />}
            {active === "sources" && (
              <SourcesEpgSection profileId={profile.id} onSourcesChanged={onSourcesChanged} />
            )}
            {active === "profile" && (
              <ProfileAboutSection
                profile={profile}
                onProfileUpdated={onProfileUpdated}
                onSwitchProfile={onSwitchProfile}
              />
            )}
          </div>
        </div>
      </motion.div>
    </motion.div>
  );
}
