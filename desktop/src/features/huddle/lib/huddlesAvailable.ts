import { isTauri } from "@tauri-apps/api/core";

/** Huddles (voice) are desktop-only for now: every huddle entry point, bar and indicator is hidden on the web build. */
export function huddlesAvailable(): boolean {
  return isTauri();
}
