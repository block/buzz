import {
  listManagedAgentRuntimes,
  startManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import type {
  ManagedAgentRuntimeLifecycle,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";
import { findManagedAgentRuntime } from "./managedAgentRuntimeStatus";

const DEFAULT_READY_TIMEOUT_MS = 60_000;
const DEFAULT_POLL_INTERVAL_MS = 500;

type RuntimeReadinessDependencies = {
  listRuntimes: () => Promise<ManagedAgentRuntimeStatus[]>;
  now: () => number;
  sleep: (milliseconds: number) => Promise<void>;
  startRuntime: (
    pubkey: string,
    relayUrl: string,
  ) => Promise<ManagedAgentRuntimeStatus>;
};

const defaultDependencies: RuntimeReadinessDependencies = {
  listRuntimes: listManagedAgentRuntimes,
  now: Date.now,
  sleep: (milliseconds) =>
    new Promise((resolve) => setTimeout(resolve, milliseconds)),
  startRuntime: startManagedAgentRuntime,
};

export function runtimeCanReceiveMessages(
  lifecycle: ManagedAgentRuntimeLifecycle,
): boolean {
  return (
    lifecycle === "listening" || lifecycle === "waking" || lifecycle === "ready"
  );
}

function runtimeFailureMessage(
  agentName: string,
  runtime: ManagedAgentRuntimeStatus,
): string {
  return runtime.error
    ? `${agentName} could not start: ${runtime.error}`
    : `${agentName} could not start.`;
}

export async function startManagedAgentRuntimeAndWait({
  agentName,
  dependencies = defaultDependencies,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  pubkey,
  relayUrl,
  timeoutMs = DEFAULT_READY_TIMEOUT_MS,
}: {
  agentName: string;
  dependencies?: RuntimeReadinessDependencies;
  pollIntervalMs?: number;
  pubkey: string;
  relayUrl: string;
  timeoutMs?: number;
}): Promise<ManagedAgentRuntimeStatus> {
  let runtime = await dependencies.startRuntime(pubkey, relayUrl);
  if (runtimeCanReceiveMessages(runtime.lifecycle)) {
    return runtime;
  }
  if (runtime.lifecycle === "failed") {
    throw new Error(runtimeFailureMessage(agentName, runtime));
  }

  const deadline = dependencies.now() + timeoutMs;
  while (dependencies.now() < deadline) {
    await dependencies.sleep(pollIntervalMs);
    runtime =
      findManagedAgentRuntime(
        await dependencies.listRuntimes(),
        pubkey,
        relayUrl,
      ) ?? runtime;

    if (runtimeCanReceiveMessages(runtime.lifecycle)) {
      return runtime;
    }
    if (runtime.lifecycle === "failed" || runtime.lifecycle === "stopped") {
      throw new Error(runtimeFailureMessage(agentName, runtime));
    }
  }

  throw new Error(
    `${agentName} did not become ready within ${Math.ceil(timeoutMs / 1000)} seconds.`,
  );
}
