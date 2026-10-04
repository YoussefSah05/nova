import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PlusIcon, SlidersHorizontal, Check } from "lucide-react";
import type { Profile, ProfileOverview } from "@/types";
import { Button } from "@/components/ui/button";
import { Reveal, Stagger, StaggerItem } from "@/lib/motion";
import { cn } from "@/lib/utils";
import { ProfileCard } from "./ProfileCard";
import { CreateProfileModal } from "./CreateProfileModal";
import { IntelligenceStatus } from "./IntelligenceStatus";
import { LaunchBackdrop } from "./LaunchBackdrop";
import { ProfileRecoveryDialog } from "./ProfileRecoveryDialog";
import { greeting } from "./launch";

interface ProfilePickerProps {
  onSelect: (profile: Profile) => void;
}

/// Most-recently-used first; never-used profiles trail by creation order.
function byRecency(a: Profile, b: Profile): number {
  return (b.lastUsedAt ?? -1) - (a.lastUsedAt ?? -1) || a.createdAt - b.createdAt;
}

export function ProfilePicker({ onSelect }: ProfilePickerProps) {
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [overviews, setOverviews] = useState<Record<string, ProfileOverview>>({});
  const [showCreate, setShowCreate] = useState(false);
  const [editing, setEditing] = useState<Profile | null>(null);
  const [managing, setManaging] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [openingId, setOpeningId] = useState<string | null>(null);
  const [recovery, setRecovery] = useState<Profile | null>(null);
  const selecting = useRef(false);

  const [attempt, setAttempt] = useState(0);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<Profile[]>("list_profiles")
      .then((list) => {
        if (cancelled) return;
        setLoadFailed(false);
        setProfiles([...list].sort(byRecency));
        // Warm each profile's at-a-glance overview (also pre-opens its DB, so the
        // subsequent select is instant). Failures are silent — the card still shows.
        for (const p of list) {
          void invoke<ProfileOverview>("get_profile_overview", { profileId: p.id })
            .then((ov) => !cancelled && setOverviews((m) => ({ ...m, [p.id]: ov })))
            .catch(() => undefined);
        }
      })
      .catch(() => {
        if (cancelled) return;
        setProfiles([]);
        setLoadFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [attempt]);

  function retryLoad() {
    setProfiles(null);
    setAttempt((n) => n + 1);
  }

  async function handleCreate(name: string, avatarColor: string) {
    const profile = await invoke<Profile>("create_profile", { name, avatarColor });
    setProfiles((prev) => [...(prev ?? []), profile].sort(byRecency));
    setLoadFailed(false);
    setShowCreate(false);
  }

  async function handleEditConfirm(name: string, avatarColor: string) {
    if (!editing) return;
    const updated = await invoke<Profile>("update_profile", {
      id: editing.id,
      name,
      avatarColor,
    });
    setProfiles(
      (prev) => prev?.map((p) => (p.id === updated.id ? updated : p)).sort(byRecency) ?? null
    );
    setEditing(null);
  }

  async function handleDelete(profile: Profile) {
    setDeletingId(profile.id);
    try {
      await invoke("delete_profile", { id: profile.id });
      setNotice(null);
      const next = profiles?.filter((p) => p.id !== profile.id) ?? null;
      setProfiles(next);
      if (next?.length === 0) setManaging(false);
    } catch {
      setNotice(`${profile.name} couldn’t be removed. Try again.`);
    } finally {
      setDeletingId(null);
    }
  }

  async function handleSelect(profile: Profile) {
    if (selecting.current) return;
    selecting.current = true;
    setOpeningId(profile.id);
    try {
      const selected = await invoke<Profile>("select_profile", { id: profile.id });
      onSelect(selected);
    } catch {
      setRecovery(profile);
    } finally {
      selecting.current = false;
      setOpeningId(null);
    }
  }

  const empty = profiles !== null && profiles.length === 0;

  return (
    <div className="relative isolate flex h-screen flex-col overflow-hidden">
      <LaunchBackdrop />

      <header
        data-tauri-drag-region="deep"
        className="relative z-10 flex h-[72px] shrink-0 items-center gap-3 pl-[88px] pr-6"
      >
        <span
          aria-hidden
          className="flex h-[30px] w-[30px] items-center justify-center rounded-[9px] bg-gradient-to-br from-[var(--color-brand-hi)] to-[var(--color-brand-lo)] font-display text-lg font-medium text-primary-foreground shadow-[0_6px_20px_rgba(224,163,62,.25)]"
        >
          N
        </span>
        <span className="font-display text-lg font-medium text-white">Nova</span>
        <div className="flex-1" />
        {profiles && profiles.length > 0 && (
          <Button
            size="sm"
            variant={managing ? "default" : "ghost"}
            disabled={openingId !== null}
            onClick={() => {
              setNotice(null);
              setManaging((m) => !m);
            }}
          >
            {managing ? (
              <Check className="h-4 w-4" strokeWidth={1.5} />
            ) : (
              <SlidersHorizontal className="h-4 w-4" strokeWidth={1.5} />
            )}
            {managing ? "Done" : "Manage profiles"}
          </Button>
        )}
      </header>

      <main className="flex min-h-0 flex-1 flex-col justify-center gap-10 px-[88px]">
        {empty ? (
          <Reveal className="flex max-w-xl flex-col items-start gap-4">
            <h1 className="font-display text-4xl font-medium tracking-tight text-white">
              Welcome to Nova
            </h1>
            <p className="text-base text-white/60">
              {loadFailed
                ? "Your profiles couldn’t be read. You can still create a new one."
                : "Create a profile to bring your playlists, progress and on-device intelligence together."}
            </p>
            {loadFailed ? (
              <div className="flex gap-3">
                <Button onClick={retryLoad}>Try again</Button>
                <Button variant="secondary" onClick={() => setShowCreate(true)}>
                  <PlusIcon className="h-4 w-4" strokeWidth={1.5} />
                  Create a new profile
                </Button>
              </div>
            ) : (
              <Button onClick={() => setShowCreate(true)}>
                <PlusIcon className="h-4 w-4" strokeWidth={1.5} />
                Create your profile
              </Button>
            )}
          </Reveal>
        ) : (
          <>
            <Reveal className="flex flex-col gap-2">
              <h1 className="font-display text-4xl font-medium tracking-tight text-white">
                {greeting()}
              </h1>
              <p className="text-base text-white/60">
                {managing ? "Choose a profile to edit or remove." : "Who’s watching tonight?"}
              </p>
              {notice && (
                <p role="alert" className="text-sm text-rose-300">
                  {notice}
                </p>
              )}
            </Reveal>

            {profiles === null ? (
              <div className="flex gap-4" aria-label="Loading profiles" role="status">
                {[0, 1].map((i) => (
                  <div
                    key={i}
                    className="nova-glass nova-shimmer h-[236px] w-[232px] rounded-2xl"
                  />
                ))}
              </div>
            ) : (
              <Stagger className="-mx-12 -mb-20 -mt-12 flex gap-4 overflow-x-auto px-12 pb-20 pt-12">
                {profiles.map((p) => (
                  <StaggerItem key={p.id} className="shrink-0">
                    <ProfileCard
                      profile={p}
                      overview={overviews[p.id]}
                      managing={managing}
                      opening={openingId === p.id}
                      dimmed={openingId !== null && openingId !== p.id}
                      onClick={handleSelect}
                      onEdit={setEditing}
                      onDelete={handleDelete}
                      isDeleting={deletingId === p.id}
                    />
                  </StaggerItem>
                ))}
                {!managing && (
                  <StaggerItem className="shrink-0">
                    <button
                      type="button"
                      onClick={() => setShowCreate(true)}
                      disabled={openingId !== null}
                      className={cn(
                        "group flex h-full min-h-[236px] w-[160px] flex-col items-start justify-end gap-3 rounded-2xl border border-white/10 bg-white/[0.02] p-5 text-left",
                        "transition-colors hover:border-white/20 hover:bg-white/[0.04] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-40"
                      )}
                    >
                      <span className="flex h-12 w-12 items-center justify-center rounded-[14px] bg-white/[0.06] text-white/60 transition-colors group-hover:text-white">
                        <PlusIcon className="h-5 w-5" strokeWidth={1.5} aria-hidden />
                      </span>
                      <span className="text-sm text-white/60 transition-colors group-hover:text-white/90">
                        Add profile
                      </span>
                    </button>
                  </StaggerItem>
                )}
              </Stagger>
            )}
          </>
        )}
      </main>

      <footer className="relative shrink-0 px-[88px] pb-10">
        <IntelligenceStatus />
      </footer>

      {recovery && (
        <ProfileRecoveryDialog
          key={recovery.id}
          profile={recovery}
          onClose={() => setRecovery(null)}
          onRecovered={(copy) =>
            setProfiles((previous) => [...(previous ?? []), copy].sort(byRecency))
          }
          onOpen={(copy) => {
            setRecovery(null);
            void handleSelect(copy);
          }}
        />
      )}
      {showCreate && (
        <CreateProfileModal onConfirm={handleCreate} onCancel={() => setShowCreate(false)} />
      )}
      {editing && (
        <CreateProfileModal
          title="Edit Profile"
          submitLabel="Save"
          initialName={editing.name}
          initialColor={editing.avatarColor}
          onConfirm={handleEditConfirm}
          onCancel={() => setEditing(null)}
        />
      )}
    </div>
  );
}
