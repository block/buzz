import { invokeTauri } from "@/shared/api/tauri";

// Seconds since the last OS-wide user input, or null where the platform has
// no supported idle API (e.g. Linux Wayland).
export function getOsIdleSeconds(): Promise<number | null> {
  return invokeTauri<number | null>("get_os_idle_seconds");
}
