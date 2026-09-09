import { Hash, Layers, LockKeyhole } from "lucide-react";
import type { Channel } from "@/shared/api/types";

const colors = [
  "bg-blue-100 text-blue-700 dark:bg-blue-900/50 dark:text-blue-200",
  "bg-violet-100 text-violet-700 dark:bg-violet-900/50 dark:text-violet-200",
  "bg-rose-100 text-rose-700 dark:bg-rose-900/50 dark:text-rose-200",
  "bg-amber-100 text-amber-700 dark:bg-amber-900/50 dark:text-amber-200",
  "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/50 dark:text-emerald-200",
  "bg-cyan-100 text-cyan-700 dark:bg-cyan-900/50 dark:text-cyan-200",
  "bg-orange-100 text-orange-700 dark:bg-orange-900/50 dark:text-orange-200",
  "bg-pink-100 text-pink-700 dark:bg-pink-900/50 dark:text-pink-200",
] as const;

/** Channel identity at the DM avatar size; omitting a channel renders the aggregate icon. */
export function PulseChannelAvatar({
  channel,
}: {
  channel?: Pick<Channel, "name" | "visibility">;
}) {
  const firstLetter = channel?.name
    .trim()
    .replace(/^#+\s*/, "")
    .toUpperCase()
    .codePointAt(0);
  const color =
    firstLetter === undefined
      ? "bg-muted text-muted-foreground"
      : colors[Math.abs(firstLetter - 65) % colors.length];
  const Icon = !channel
    ? Layers
    : channel.visibility === "private"
      ? LockKeyhole
      : Hash;
  return (
    <span
      aria-hidden
      className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-full ${color}`}
    >
      <Icon className="h-4 w-4" />
    </span>
  );
}
