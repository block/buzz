import type { useUserProfileQuery } from "@/features/profile/hooks";

/** Resolved profile document for the viewed identity (kind:0 metadata, etc.). */
export type ProfileSummaryData = ReturnType<typeof useUserProfileQuery>["data"];

/** Free-form user status line (emoji + text), or absent. */
export type ProfileSummaryUserStatus =
  | {
      text: string;
      emoji: string;
    }
  | null
  | undefined;
