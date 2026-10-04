import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PlusIcon, SlidersHorizontal, Check } from "lucide-react";
import type { Profile, ProfileOverview } from "@/types";
import { Button } from "@/components/ui/button";
import { DragRegion } from "@/components/DragRegion";
import { Reveal, Stagger, StaggerItem } from "@/lib/motion";
import { ProfileCard } from "./ProfileCard";
import { CreateProfileModal } from "./CreateProfileModal";
import { AuroraBackdrop } from "./AuroraBackdrop";
import { ProfileRecoveryDialog } from "./ProfileRecoveryDialog";

interface ProfilePickerProps {
  onSelect: (profile: Profile) => void;
}

/// Most-recently-used first; never-used profiles trail by creation order.
function byRecency(a: Profile, b: Profile): number {
  return (b.lastUsedAt ?? -1) - (a.lastUsedAt ?? -1) || a.createdAt - b.createdAt;
}

export function ProfilePicker({ onSelect }: ProfilePickerProps) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [overviews, setOverviews] = useState<Record<string, ProfileOverview>>({});
  const [showCreate, setShowCreate] = useState(false);
  const [editing, setEditing] = useState<Profile | null>(null);
  const [managing, setManaging] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [recovery, setRecovery] = useState<Profile | null>(null);
  const selecting = useRef(false);

  useEffect(() => {
    let cancelled = false;
    void invoke<Profile[]>("list_profiles").then((list) => {
      if (cancelled) return;
      setProfiles([...list].sort(byRecency));
      // Warm each profile's at-a-glance overview (also pre-opens its DB, so the
      // subsequent select is instant). Failures are silent — the card still shows.
      for (const p of list) {
        void invoke<ProfileOverview>("get_profile_overview", { profileId: p.id })
          .then((ov) => !cancelled && setOverviews((m) => ({ ...m, [p.id]: ov })))
          .catch(() => undefined);
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  async function handleCreate(name: string, avatarColor: string) {
    const profile = await invoke<Profile>("create_profile", { name, avatarColor });
    setProfiles((prev) => [...prev, profile].sort(byRecency));
    setShowCreate(false);
  }

  async function handleEditConfirm(name: string, avatarColor: string) {
    if (!editing) return;
    const updated = await invoke<Profile>("update_profile", {
      id: editing.id,
      name,
      avatarColor,
    });
    setProfiles((prev) => prev.map((p) => (p.id === updated.id ? updated : p)).sort(byRecency));
    setEditing(null);
  }

  async function handleDelete(profile: Profile) {
    setDeletingId(profile.id);
    try {
      await invoke("delete_profile", { id: profile.id });
      setProfiles((prev) => prev.filter((p) => p.id !== profile.id));
    } finally {
      setDeletingId(null);
    }
  }

  async function handleSelect(profile: Profile) {
    if (selecting.current) return;
    selecting.current = true;
    try {
      const selected = await invoke<Profile>("select_profile", { id: profile.id });
      onSelect(selected);
    } catch {
      setRecovery(profile);
    } finally {
      selecting.current = false;
    }
  }

  return (
    <div className="relative flex min-h-screen flex-col items-center justify-center gap-10 px-6">
      <AuroraBackdrop />
      <DragRegion />

      <Reveal>
        <h1 className="font-display text-center text-5xl font-semibold tracking-tight text-white">
          Who's watching?
        </h1>
      </Reveal>

      <Stagger className="flex max-w-3xl flex-wrap justify-center gap-6">
        {profiles.map((p) => (
          <StaggerItem key={p.id}>
            <ProfileCard
              profile={p}
              overview={overviews[p.id]}
              managing={managing}
              onClick={handleSelect}
              onEdit={setEditing}
              onDelete={handleDelete}
              isDeleting={deletingId === p.id}
            />
          </StaggerItem>
        ))}

        <StaggerItem>
          <button
            onClick={() => setShowCreate(true)}
            className="group flex h-full min-h-[180px] w-44 cursor-pointer flex-col items-center justify-center gap-3 rounded-2xl border-2 border-dashed border-white/15 transition-all hover:border-primary/60 hover:bg-white/[0.03]"
          >
            <div className="flex h-16 w-16 items-center justify-center rounded-full border-2 border-dashed border-white/20 text-white/40 transition-all group-hover:scale-105 group-hover:border-primary/60 group-hover:text-white/80">
              <PlusIcon className="h-7 w-7" />
            </div>
            <span className="text-sm font-medium text-white/40 transition-colors group-hover:text-white/70">
              Add Profile
            </span>
          </button>
        </StaggerItem>
      </Stagger>

      {profiles.length > 0 && (
        <Reveal>
          <Button
            variant={managing ? "default" : "secondary"}
            onClick={() => setManaging((m) => !m)}
          >
            {managing ? <Check className="h-4 w-4" /> : <SlidersHorizontal className="h-4 w-4" />}
            {managing ? "Done" : "Manage profiles"}
          </Button>
        </Reveal>
      )}

      {recovery && (
        <ProfileRecoveryDialog
          key={recovery.id}
          profile={recovery}
          onClose={() => setRecovery(null)}
          onRecovered={(copy) => setProfiles((previous) => [...previous, copy].sort(byRecency))}
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
