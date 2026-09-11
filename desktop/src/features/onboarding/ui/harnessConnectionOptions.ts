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
