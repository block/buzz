import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

const HUDDLE_WINDOW_LABEL_PREFIX = "huddle-";
const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Returns the ephemeral channel id only for a dedicated huddle room window. */
export function huddleWindowChannelId(): string | null {
  if (!isTauri()) return null;

  let label: string;
  try {
    label = getCurrentWindow().label;
  } catch {
    // Browser previews can expose the Tauri IPC mock without window metadata.
    return null;
  }
  if (!label.startsWith(HUDDLE_WINDOW_LABEL_PREFIX)) return null;

  const channelId = label.slice(HUDDLE_WINDOW_LABEL_PREFIX.length);
  return UUID_PATTERN.test(channelId) ? channelId : null;
}
