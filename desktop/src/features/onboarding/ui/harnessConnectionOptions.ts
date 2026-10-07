import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

export type HarnessConnectionMethod = "subscription" | "api";

const SUBSCRIPTION_RUNTIME_IDS = new Set([
  "claude",
  "codex",
  "cursor",
  "devin",
  "amp",
]);

const API_RUNTIME_IDS = new Set([
  "buzz-agent",
  "goose",
  "omp",
  "grok",
  "opencode",
  "kimi",
  "hermes",
  "openclaw",
]);

export function runtimeSupportsConnectionMethod(
  runtimeId: string,
  method: HarnessConnectionMethod,
) {
  return (
    method === "subscription" ? SUBSCRIPTION_RUNTIME_IDS : API_RUNTIME_IDS
  ).has(runtimeId);
}

export function runtimeUnavailableDescription(
  runtime: AcpRuntimeCatalogEntry,
): string {
  return runtime.availability === "adapter_outdated"
    ? `${runtime.label} needs an ACP adapter update.`
    : `${runtime.label} is not detected on this computer.`;
}

export function getRuntimesForConnectionMethod(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  method: HarnessConnectionMethod,
) {
  return runtimes.filter((runtime) =>
    runtimeSupportsConnectionMethod(runtime.id, method),
  );
}

/**
 * Keeps the installed and unavailable groups contiguous so the list can render
 * a single "Not installed" divider. API-first choices stay prioritized within
 * their availability group instead of splitting that divider in two.
 */
export function orderRuntimesForConnectionMethod(
  runtimes: readonly AcpRuntimeCatalogEntry[],
  method: HarnessConnectionMethod,
) {
  const priority = (runtime: AcpRuntimeCatalogEntry) => {
    if (method !== "api") return 0;
    if (runtime.id === "buzz-agent") return 0;
    if (runtime.id === "goose") return 1;
    return 2;
  };

  return [...getRuntimesForConnectionMethod(runtimes, method)].sort(
    (left, right) => {
      const availabilityDifference =
        Number(left.availability !== "available") -
        Number(right.availability !== "available");
      return availabilityDifference || priority(left) - priority(right);
    },
  );
}
