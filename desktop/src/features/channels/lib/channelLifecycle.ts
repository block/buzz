import { formatTtlDuration } from "@/features/channels/lib/ephemeralChannel";
import { i18n } from "@/i18n";

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
  if (lifecycle === "project") return i18n.t("channels.lifecycle.project");
  if (lifecycle === "temporary" && ttlSeconds != null) {
    return i18n.t("channels.lifecycle.temporary-duration", {
      duration: formatTtlDuration(ttlSeconds),
    });
  }
  if (lifecycle === "temporary") return i18n.t("channels.lifecycle.temporary");
  return i18n.t("channels.lifecycle.ongoing");
}
