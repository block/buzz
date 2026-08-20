import { Bot, Inbox } from "lucide-react";

import type { GoToAcceleratorItem } from "@/features/navigation/lib/goToAccelerators";

export type GoToDestinationId = "inbox" | "agents";

/**
 * A top-level app area reachable from the ⌘G "Go to" palette.
 *
 * Ordering here defines both the visual order and the bare-digit positional
 * accelerators (row 1, row 2, …). The `mnemonic` is a stable, hand-assigned
 * ⌘/Ctrl+letter accelerator that stays constant as the list grows, so muscle
 * memory never breaks. Add `feature` to gate an area behind a preview flag —
 * gated-off areas are hidden so the palette never navigates to a dead end.
 */
export type GoToDestination = GoToAcceleratorItem & {
  id: GoToDestinationId;
  icon: React.ComponentType<{ className?: string }>;
};

export const GO_TO_DESTINATIONS: readonly GoToDestination[] = [
  {
    id: "inbox",
    label: "Inbox",
    icon: Inbox,
    mnemonic: "I",
    keywords: ["home", "feed", "activity", "notifications"],
  },
  {
    id: "agents",
    label: "Agents",
    icon: Bot,
    mnemonic: "A",
    keywords: ["bots", "directory", "job board"],
  },
];
