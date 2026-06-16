import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Profile } from "@/types";
import { ProfileCard } from "./ProfileCard";
import { CreateProfileModal } from "./CreateProfileModal";
import { PlusIcon } from "lucide-react";
import { DragRegion } from "@/components/DragRegion";
import { Reveal, Stagger, StaggerItem } from "@/lib/motion";

interface ProfilePickerProps {
  onSelect: (profile: Profile) => void;
}

export function ProfilePicker({ onSelect }: ProfilePickerProps) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [showCreate, setShowCreate] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [selectError, setSelectError] = useState<string | null>(null);

  useEffect(() => {
    invoke<Profile[]>("list_profiles").then(setProfiles);
  }, []);

  async function handleCreate(name: string, avatarColor: string) {
    const profile = await invoke<Profile>("create_profile", { name, avatarColor });
    setProfiles((prev) => [...prev, profile]);
    setShowCreate(false);
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
    setSelectError(null);
    try {
      const selected = await invoke<Profile>("select_profile", { id: profile.id });
      onSelect(selected);
    } catch (e) {
      // Surface the failure instead of silently doing nothing (e.g. the
      // profile DB can't be opened).
      setSelectError(typeof e === "string" ? e : "Couldn't open this profile.");
    }
  }

  return (
    <div className="relative flex min-h-screen flex-col items-center justify-center gap-10 bg-neutral-950">
      <DragRegion />
      <Reveal>
        <h1 className="font-display text-4xl font-semibold text-white tracking-tight">
          Who's watching?
        </h1>
      </Reveal>

      {selectError && (
        <p className="-mt-6 max-w-md text-center text-sm text-rose-300">{selectError}</p>
      )}

      <Stagger className="flex flex-wrap justify-center gap-8">
        {profiles.map((p) => (
          <StaggerItem key={p.id}>
            <ProfileCard
              profile={p}
              onClick={handleSelect}
              onDelete={handleDelete}
              isDeleting={deletingId === p.id}
            />
          </StaggerItem>
        ))}

        <StaggerItem>
          <button
            onClick={() => setShowCreate(true)}
            className="group flex flex-col items-center gap-3 cursor-pointer"
          >
            <div className="flex h-20 w-20 items-center justify-center rounded-full border-2 border-dashed border-white/20 text-white/40 transition-all group-hover:border-primary/60 group-hover:text-white/80 group-hover:scale-105">
              <PlusIcon className="h-8 w-8" />
            </div>
            <span className="text-sm font-medium text-white/40 group-hover:text-white/70 transition-colors">
              Add Profile
            </span>
          </button>
        </StaggerItem>
      </Stagger>

      {showCreate && (
        <CreateProfileModal onConfirm={handleCreate} onCancel={() => setShowCreate(false)} />
      )}
    </div>
  );
}
