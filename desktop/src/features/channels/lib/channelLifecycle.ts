// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
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
  if (lifecycle === "project") return uiT("Project");
  if (lifecycle === "temporary" && ttlSeconds != null) {
    return uiT("Temporary · {0}", { p0: formatTtlDuration(ttlSeconds) });
  }
  if (lifecycle === "temporary") return uiT("Temporary");
  return uiT("Ongoing");
}
