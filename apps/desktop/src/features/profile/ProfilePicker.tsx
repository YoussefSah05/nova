import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Profile } from "@/types";
import { ProfileCard } from "./ProfileCard";
import { CreateProfileModal } from "./CreateProfileModal";
import { PlusIcon } from "lucide-react";

interface ProfilePickerProps {
  onSelect: (profile: Profile) => void;
}

export function ProfilePicker({ onSelect }: ProfilePickerProps) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [showCreate, setShowCreate] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);

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
    const selected = await invoke<Profile>("select_profile", { id: profile.id });
    onSelect(selected);
  }

  return (
    <div className="flex min-h-screen flex-col items-center justify-center gap-10 bg-neutral-950">
      <h1 className="text-3xl font-semibold text-white tracking-tight">Who's watching?</h1>

      <div className="flex flex-wrap justify-center gap-8">
        {profiles.map((p) => (
          <ProfileCard
            key={p.id}
            profile={p}
            onClick={handleSelect}
            onDelete={handleDelete}
            isDeleting={deletingId === p.id}
          />
        ))}

        <button
          onClick={() => setShowCreate(true)}
          className="group flex flex-col items-center gap-3 cursor-pointer"
        >
          <div className="flex h-20 w-20 items-center justify-center rounded-full border-2 border-dashed border-white/20 text-white/40 transition-all group-hover:border-white/60 group-hover:text-white/80 group-hover:scale-105">
            <PlusIcon className="h-8 w-8" />
          </div>
          <span className="text-sm font-medium text-white/40 group-hover:text-white/70 transition-colors">
            Add Profile
          </span>
        </button>
      </div>

      {showCreate && (
        <CreateProfileModal onConfirm={handleCreate} onCancel={() => setShowCreate(false)} />
      )}
    </div>
  );
}
