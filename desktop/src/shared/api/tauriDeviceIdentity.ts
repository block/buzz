import { invokeTauri } from "@/shared/api/tauri";
import type { DeviceIdentity } from "@/shared/api/types";

/** Read this install's device identity, minting one on first call. */
export async function getDeviceIdentity(): Promise<DeviceIdentity> {
  return invokeTauri<DeviceIdentity>("get_device_identity");
}

/**
 * Rename this device.
 *
 * The backend trims, rejects control characters, caps the label at 32
 * characters, and republishes the label on every local agent's kind:30177
 * event, so callers need no follow-up write.
 */
export async function setDeviceLabel(label: string): Promise<DeviceIdentity> {
  return invokeTauri<DeviceIdentity>("set_device_label", { label });
}
