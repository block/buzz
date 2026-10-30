import { formatTtlDuration } from "@/features/channels/lib/ephemeralChannel";

export type ChannelLifecycle = "ongoing" | "temporary" | "project";

export function channelLifecycle(input: {
  projectHome: boolean;
  temporary: boolean;
}): ChannelLifecycle {
  if (input.projectHome) return "project";
  return input.temporary ? "temporary" : "ongoing";
}

export function channelLifecycleLabel(
  lifecycle: ChannelLifecycle,
  ttlSeconds: number | null,
): string {
  if (lifecycle === "project") return "Project";
  if (lifecycle === "temporary" && ttlSeconds != null) {
    return `Temporary · ${formatTtlDuration(ttlSeconds)}`;
  }
  if (lifecycle === "temporary") return "Temporary";
  return "Ongoing";
}
