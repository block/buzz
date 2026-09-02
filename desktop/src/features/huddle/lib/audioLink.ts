// Mirrors `AudioLink` in src-tauri/src/huddle/state.rs.
export type HuddleAudioLink =
  | { status: "live" }
  | { status: "reconnecting"; attempt: number; draining: boolean }
  | { status: "lost" };

/**
 * Banner copy for a degraded audio link, or null while audio is live. Rust
 * owns the retry loop; the renderer only reports what it is doing.
 */
export function audioLinkNotice(
  link: HuddleAudioLink | undefined,
): { tone: "info" | "error"; message: string } | null {
  switch (link?.status) {
    case "reconnecting":
      return {
        tone: "info",
        message: link.draining
          ? "The huddle relay is restarting. Reconnecting…"
          : "Audio dropped. Reconnecting…",
      };
    case "lost":
      return {
        tone: "error",
        message: "Couldn’t reconnect audio. Leave and rejoin the huddle.",
      };
    default:
      return null;
  }
}
