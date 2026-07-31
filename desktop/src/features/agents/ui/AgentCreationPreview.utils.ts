import type * as React from "react";

export function isAvatarFileDrag(event: React.DragEvent<HTMLElement>) {
  return Array.from(event.dataTransfer.types).includes("Files");
}

export const AVATAR_APPLY_MOTION_TRANSITION = {
  duration: 0.14,
  ease: [0.23, 1, 0.32, 1],
} as const;

export type AvatarTab = "image" | "emoji";

export type EmojiMartEmoji = {
  native?: string;
};
