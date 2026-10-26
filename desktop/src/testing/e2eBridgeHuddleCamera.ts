import { emit, listen } from "@tauri-apps/api/event";
import {
  type CameraDevice,
  listHuddleCameras,
} from "@/features/huddle/lib/huddleCamera";

/** Represent the absent main window in companion-only browser fixtures. */
export async function installMockHuddleCameraOwner(
  windowLabel: string | undefined,
  channel: () => string | null,
) {
  if (!windowLabel?.startsWith("huddle-")) return;
  await listen<{
    channel: string;
    type: string;
    requestId: string;
  }>("huddle-camera-devices", async ({ payload }) => {
    if (payload.type !== "request" || payload.channel !== channel()) return;
    let result: { cameras: CameraDevice[] } | { error: string };
    try {
      const devices = navigator.mediaDevices;
      if (!devices?.enumerateDevices) result = { cameras: [] };
      else result = { cameras: await listHuddleCameras(devices) };
    } catch (failure) {
      result = { error: `Could not list cameras: ${String(failure)}` };
    }
    if (payload.channel !== channel()) return;
    await emit("huddle-camera-devices", {
      channel: payload.channel,
      type: "result",
      requestId: payload.requestId,
      result,
    });
  });
  const active = channel();
  if (active)
    await emit("huddle-camera-devices", { channel: active, type: "ready" });
}
