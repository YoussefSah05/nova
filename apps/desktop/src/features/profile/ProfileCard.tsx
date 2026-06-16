import { cn } from "@/lib/utils";
import { motion, pressable } from "@/lib/motion";
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
      <motion.div
        {...pressable}
        className={cn(
          "relative flex h-20 w-20 items-center justify-center rounded-full text-2xl font-semibold text-white",
          "ring-2 ring-transparent transition-[box-shadow,ring-color] duration-200",
          "group-hover:ring-primary group-hover:[box-shadow:var(--shadow-brand-glow)]"
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
      </motion.div>
      <span className="text-sm font-medium text-white/80 group-hover:text-white transition-colors">
        {profile.name}
      </span>
    </div>
  );
}
