//! Settings share one accessible dialog; section changes preserve model jobs.
import { useRef, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { Captions, Cpu, HardDrive, Palette, Sparkles, Tv, UserCog, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Reveal } from "@/lib/motion";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { Profile } from "@/types";
import { PlaybackCaptionsSection } from "./sections/PlaybackCaptionsSection";
import { ModelsSection } from "./sections/ModelsSection";
import { StorageSection } from "./sections/StorageSection";
import { EnrichmentSection } from "./sections/EnrichmentSection";
import { AppearanceSection } from "./sections/AppearanceSection";
import { SourcesEpgSection } from "./sections/SourcesEpgSection";
import { ProfileAboutSection } from "./sections/ProfileAboutSection";

const SECTIONS = [
  {
    id: "playback",
    label: "Playback & captions",
    description: "Make every watch feel right.",
    icon: Captions,
  },
  {
    id: "appearance",
    label: "Appearance",
    description: "A little more you, throughout Nova.",
    icon: Palette,
  },
  {
    id: "models",
    label: "Models",
    description: "On-device intelligence, ready when you need it.",
    icon: Cpu,
  },
  {
    id: "storage",
    label: "Storage",
    description: "See what Nova keeps on this Mac.",
    icon: HardDrive,
  },
  {
    id: "catalog",
    label: "Catalog",
    description: "Choose how your titles and artwork are enriched.",
    icon: Sparkles,
  },
  {
    id: "sources",
    label: "Playlists & guide",
    description: "Keep your playlists and programme guides up to date.",
    icon: Tv,
  },
  {
    id: "profile",
    label: "Profile & about",
    description: "Your profile, your preferences.",
    icon: UserCog,
  },
] as const;

export function SettingsModal({
  profile,
  onClose,
  onSwitchProfile,
  onProfileUpdated,
  onSourcesChanged,
  onAddSource,
}: {
  profile: Profile;
  onClose: () => void;
  onSwitchProfile: () => void;
  onProfileUpdated: (p: Profile) => void;
  onSourcesChanged: () => void;
  onAddSource: () => void;
}) {
  const [active, setActive] = useState("playback");
  const [previousFocus] = useState(() => document.activeElement);
  const handoff = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const section = SECTIONS.find((s) => s.id === active) ?? SECTIONS[0];
  function navigate(next: string) {
    setActive(next);
    scrollRef.current?.scrollTo({ top: 0 });
  }
  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-background/80 backdrop-blur-sm" />
        <Dialog.Content
          aria-modal="true"
          onInteractOutside={(e) => e.preventDefault()}
          onCloseAutoFocus={(e) => {
            e.preventDefault();
            if (handoff.current) return;
            const target =
              previousFocus instanceof HTMLElement &&
              previousFocus.isConnected &&
              previousFocus !== document.body
                ? previousFocus
                : document.querySelector<HTMLElement>('[aria-label="Profile and settings"]');
            target?.focus();
          }}
          className="fixed left-1/2 top-1/2 z-50 h-[calc(100dvh-48px)] max-h-[800px] w-[calc(100vw-48px)] max-w-5xl -translate-x-1/2 -translate-y-1/2 overflow-hidden rounded-2xl border border-border bg-background shadow-2xl outline-none"
        >
          <Tabs
            value={active}
            onValueChange={navigate}
            orientation="vertical"
            className="flex h-full min-h-0"
          >
            <aside className="flex w-52 shrink-0 flex-col overflow-y-auto border-r border-border bg-card/50 p-4">
              <Dialog.Title className="px-3 pb-1 pt-3 font-display text-[22px] font-medium">
                Settings
              </Dialog.Title>
              <Dialog.Description className="px-3 pb-6 text-xs text-muted-foreground">
                For {profile.name} · on this Mac
              </Dialog.Description>
              <TabsList
                aria-label="Settings sections"
                className="h-auto shrink-0 flex-col items-stretch gap-1 rounded-none border-0 bg-transparent p-0"
              >
                {SECTIONS.map(({ id, label, icon: Icon }) => (
                  <TabsTrigger
                    key={id}
                    value={id}
                    className={cn(
                      "justify-start gap-3 rounded-lg px-3 py-3 text-left text-sm font-normal normal-case",
                      "data-[state=active]:bg-primary/10 data-[state=active]:text-primary"
                    )}
                  >
                    <Icon size={16} strokeWidth={1.5} aria-hidden />
                    <span>{label}</span>
                  </TabsTrigger>
                ))}
              </TabsList>
              <p className="mt-auto px-3 pt-6 text-xs leading-relaxed text-muted-foreground">
                Profile preferences stay with {profile.name}. Models and artwork are shared on this
                Mac.
              </p>
            </aside>
            <div className="flex min-w-0 flex-1 flex-col">
              <header className="flex items-start justify-between gap-4 border-b border-border p-6">
                <div>
                  <h2 className="font-display text-[28px] font-medium">{section.label}</h2>
                  <p className="mt-2 text-sm text-muted-foreground">{section.description}</p>
                </div>
                <Dialog.Close
                  className="rounded-full p-2 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  aria-label="Close settings"
                >
                  <X size={20} strokeWidth={1.5} aria-hidden />
                </Dialog.Close>
              </header>
              <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto p-6">
                <Reveal key={`${profile.id}:${active}`}>
                  <TabsContent value="appearance">
                    <AppearanceSection profileId={profile.id} />
                  </TabsContent>
                  <TabsContent value="playback">
                    <PlaybackCaptionsSection profileId={profile.id} />
                  </TabsContent>
                  <TabsContent value="models">
                    <ModelsSection />
                  </TabsContent>
                  <TabsContent value="storage">
                    <StorageSection profileId={profile.id} />
                  </TabsContent>
                  <TabsContent value="catalog">
                    <EnrichmentSection
                      profileId={profile.id}
                      onOpenModels={() => navigate("models")}
                    />
                  </TabsContent>
                  <TabsContent value="sources">
                    <SourcesEpgSection
                      profileId={profile.id}
                      onSourcesChanged={onSourcesChanged}
                      onAddSource={() => {
                        handoff.current = true;
                        onAddSource();
                      }}
                    />
                  </TabsContent>
                  <TabsContent value="profile">
                    <ProfileAboutSection
                      profile={profile}
                      onProfileUpdated={onProfileUpdated}
                      onSwitchProfile={() => {
                        handoff.current = true;
                        onSwitchProfile();
                      }}
                    />
                  </TabsContent>
                </Reveal>
              </div>
            </div>
          </Tabs>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
