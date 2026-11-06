import { invokeTauri } from "@/shared/api/tauri";
import type { DeviceIdentity } from "@/shared/api/types";

/** Read this install's device identity, minting one on first call. */
export async function getDeviceIdentity(): Promise<DeviceIdentity> {
  return invokeTauri<DeviceIdentity>("get_device_identity");
}

/**
 * Rename this device.
 *
 * The backend trims and rejects an empty, over-long (>32 char), or
 * invisible-character-bearing label — including the zero-width and bidi
 * codepoints `char::is_control` misses — then republishes the label on the
 * active community's local agents, so callers need no follow-up write. The
 * owner's other communities pick it up when next activated.
 */
export async function setDeviceLabel(label: string): Promise<DeviceIdentity> {
  return invokeTauri<DeviceIdentity>("set_device_label", { label });
}

/**
 * The OS host name, offered as a suggested device name.
 *
 * `null` when the host name is unusable under the label policy. This is only a
 * suggestion: a device's name starts opaque (`device-xxxxxxxx`) and the host
 * name — which routinely contains a real person's name — is never published
 * until the owner applies it.
 */
export async function getDeviceNameSuggestion(): Promise<string | null> {
  return invokeTauri<string | null>("get_device_name_suggestion");
}
