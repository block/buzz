import { UserRound } from "lucide-react";

import { getInitials } from "@/shared/lib/initials";
import { cn } from "@/shared/lib/cn";

const IDENTITY_INITIAL_AVATAR_CLASS_NAMES = [
  "bg-muted text-foreground",
  "bg-secondary text-secondary-foreground",
  "bg-accent text-accent-foreground",
  "bg-card text-card-foreground",
  "bg-popover text-popover-foreground",
  "bg-background text-foreground",
] as const;

type IdentityInitialsAvatarProps = {
  className?: string;
  colorIndex?: number;
  colorSeed?: string;
  label: string;
  size: number;
};

export function IdentityInitialsAvatar({
  className,
  colorIndex,
  colorSeed,
  label,
  size,
}: IdentityInitialsAvatarProps) {
  const initials = getInitials(label);
  const seed = colorSeed ?? (label || "agent");
  const paletteIndex = colorIndex ?? getStableColorIndex(seed);
  const colorClassName =
    IDENTITY_INITIAL_AVATAR_CLASS_NAMES[
      paletteIndex % IDENTITY_INITIAL_AVATAR_CLASS_NAMES.length
    ];
  const textSizeClassName = size >= 80 ? "text-3xl" : "text-xl";

  return (
    <span
      className={cn(
        "flex h-full w-full items-center justify-center rounded-full border-[3px] border-background font-semibold shadow-sm",
        colorClassName,
        textSizeClassName,
        className,
      )}
    >
      {initials.length > 0 ? initials : <UserRound className="h-8 w-8" />}
    </span>
  );
}

function getStableColorIndex(seed: string) {
  let hash = 0;
  for (let index = 0; index < seed.length; index += 1) {
    hash = (hash * 31 + seed.charCodeAt(index)) >>> 0;
  }
  return hash;
}
