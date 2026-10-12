// Error-state copy for slice B (docs/pwa-bridge-slice-b-c-design.md "Web error states").
import type { DesktopPresence, RemoteInfo } from "./protocol";

export type RemoteProblem = "none" | "desktop_unreachable" | "remote_off" | "terminals_off" | "tailscale_device" | "declined" | "other";

/** What to tell the user before/after trying to connect. `failure` is a RemoteError code from a failed attempt. */
export function describeRemote(
  presence: DesktopPresence,
  info: RemoteInfo | undefined,
  failure?: string,
): { problem: RemoteProblem; message: string } {
  const name = info?.name || "your desktop";
  if (failure === "declined" || failure === "user_rejected")
    return { problem: "declined", message: "Approval was declined in your signer." };
  if (presence !== "online")
    return { problem: "desktop_unreachable", message: "Your desktop isn't reachable (asleep or app closed)." };
  if (!info || info.status === "off")
    return {
      problem: "remote_off",
      message: `Remote access isn't available on ${name}: the direct connection is off. Turn on "Allow remote agent management" (Settings → Agents) or "Allow remote terminal" (Settings → Buzz Term) on that computer.${info?.reason ? ` ${info.reason}` : ""}`,
    };
  if (failure === "terminals_disabled")
    return { problem: "terminals_off", message: `"Allow remote terminal" is off on ${name} (Settings → Buzz Term on that computer).` };
  if (failure === "unreachable" || failure === "timeout")
    return { problem: "tailscale_device", message: "Terminals need a direct connection over your tailnet. Connect to Tailscale on this device (no host is reachable right now)." };
  if (failure && failure !== "closed")
    return { problem: "other", message: `The desktop refused the request (${failure}).` };
  if (failure === "closed") return { problem: "desktop_unreachable", message: "The connection to your desktop was closed." };
  return { problem: "none", message: "" };
}
