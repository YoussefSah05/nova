import { cn } from "@/lib/utils";
import type { Profile } from "@/types";

interface ProfileCardProps {
  profile: Profile;
  onClick: (profile: Profile) => void;
  onDelete?: (profile: Profile) => void;
  isDeleting?: boolean;
}

export function ProfileCard({ profile, onClick, onDelete, isDeleting }: ProfileCardProps) {
  const initials = profile.name
    .split(" ")
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();

  return (
    <div
      className="group flex flex-col items-center gap-3 cursor-pointer"
      onClick={() => onClick(profile)}
    >
      <div
        className={cn(
          "relative flex h-20 w-20 items-center justify-center rounded-full text-2xl font-semibold text-white",
          "ring-2 ring-transparent transition-all duration-200",
          "group-hover:ring-white group-hover:scale-105"
        )}
        style={{ backgroundColor: profile.avatarColor }}
      >
        {initials}
        {onDelete && (
          <button
            className={cn(
              "absolute -top-1 -right-1 flex h-5 w-5 items-center justify-center rounded-full",
              "bg-neutral-800 text-white opacity-0 transition-opacity group-hover:opacity-100",
              "hover:bg-red-600 text-xs"
            )}
            onClick={(e) => {
              e.stopPropagation();
              onDelete(profile);
            }}
            disabled={isDeleting}
            aria-label={`Delete ${profile.name}`}
          >
            ×
          </button>
        )}
      </div>
      <span className="text-sm font-medium text-white/80 group-hover:text-white transition-colors">
        {profile.name}
      </span>
    </div>
  );
}
