import * as React from "react";
import {
  abortManagedAgentIsolation,
  getPreparedManagedAgentIsolation,
  prepareManagedAgentIsolation,
} from "@/shared/api/tauriManagedAgents";
import type {
  FilesystemIsolationProfile,
  ManagedAgent,
  PreparedFilesystemIsolation,
} from "@/shared/api/types";
import { isMacPlatform } from "@/shared/lib/platform";

export type FilesystemIsolationFieldProps = {
  available: boolean;
  enabled: boolean;
  readOnlyRoots: string;
  prepared: PreparedFilesystemIsolation | null;
  prepareError: string | null;
  preparePending: boolean;
  canPrepare: boolean;
  onEnabledChange: (value: boolean) => void;
  onReadOnlyRootsChange: (value: string) => void;
  onPrepare: () => Promise<void>;
  onAbort: () => Promise<void>;
};

export function parseIsolationReadOnlyRoots(value: string): string[] {
  return Array.from(
    new Set(
      value
        .split("\n")
        .map((root) => root.trim())
        .filter(Boolean),
    ),
  );
}

export function isolationReadOnlyRootsAreAbsolute(roots: string[]): boolean {
  return roots.every((root) => root.startsWith("/"));
}

export function filesystemIsolationIsAvailable(
  backendType: ManagedAgent["backend"]["type"],
  macPlatform: boolean,
): boolean {
  return backendType === "local" && macPlatform;
}

export function filesystemIsolationProfilesEqual(
  left: FilesystemIsolationProfile | null,
  right: FilesystemIsolationProfile | null,
): boolean {
  if (left === null || right === null) return left === right;
  if (left.mode !== right.mode) return false;
  return (
    [...new Set(left.readOnlyRoots)].sort().join("\n") ===
    [...new Set(right.readOnlyRoots)].sort().join("\n")
  );
}

export function resolveFilesystemIsolationUpdate(
  enabled: boolean,
  readOnlyRoots: string,
  current: FilesystemIsolationProfile | null,
): FilesystemIsolationProfile | null | undefined {
  const next: FilesystemIsolationProfile | null = enabled
    ? {
        mode: "ephemeral",
        readOnlyRoots: parseIsolationReadOnlyRoots(readOnlyRoots),
      }
    : null;
  return filesystemIsolationProfilesEqual(next, current) ? undefined : next;
}

export function useFilesystemIsolationDraft(
  agent: ManagedAgent,
  open: boolean,
): {
  fieldProps: FilesystemIsolationFieldProps;
  valid: boolean;
  update: FilesystemIsolationProfile | null | undefined;
} {
  const [enabled, setEnabled] = React.useState(
    agent.filesystemIsolation?.mode === "ephemeral",
  );
  const [readOnlyRoots, setReadOnlyRoots] = React.useState(
    agent.filesystemIsolation?.readOnlyRoots.join("\n") ?? "",
  );
  const [prepared, setPrepared] =
    React.useState<PreparedFilesystemIsolation | null>(null);
  const [prepareError, setPrepareError] = React.useState<string | null>(null);
  const [preparePending, setPreparePending] = React.useState(false);

  // Polling refreshes must not wipe an in-progress edit.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset only on open or identity switch
  React.useEffect(() => {
    if (!open) return;
    setEnabled(agent.filesystemIsolation?.mode === "ephemeral");
    setReadOnlyRoots(agent.filesystemIsolation?.readOnlyRoots.join("\n") ?? "");
    setPrepareError(null);
    void getPreparedManagedAgentIsolation(agent.pubkey)
      .then(setPrepared)
      .catch((error) =>
        setPrepareError(error instanceof Error ? error.message : String(error)),
      );
  }, [open, agent.pubkey]);

  const parsedRoots = parseIsolationReadOnlyRoots(readOnlyRoots);

  const update = resolveFilesystemIsolationUpdate(
    enabled,
    readOnlyRoots,
    agent.filesystemIsolation,
  );
  const canPrepare =
    enabled && agent.filesystemIsolation !== null && update === undefined;

  async function prepare() {
    setPreparePending(true);
    setPrepareError(null);
    try {
      setPrepared(await prepareManagedAgentIsolation(agent.pubkey));
    } catch (error) {
      setPrepareError(error instanceof Error ? error.message : String(error));
    } finally {
      setPreparePending(false);
    }
  }

  async function abort() {
    if (!prepared) return;
    setPreparePending(true);
    setPrepareError(null);
    try {
      await abortManagedAgentIsolation(agent.pubkey, prepared.runId);
      setPrepared(null);
    } catch (error) {
      setPrepareError(error instanceof Error ? error.message : String(error));
    } finally {
      setPreparePending(false);
    }
  }

  return {
    fieldProps: {
      available: filesystemIsolationIsAvailable(
        agent.backend.type,
        isMacPlatform(),
      ),
      enabled,
      readOnlyRoots,
      prepared,
      prepareError,
      preparePending,
      canPrepare,
      onEnabledChange: setEnabled,
      onReadOnlyRootsChange: setReadOnlyRoots,
      onPrepare: prepare,
      onAbort: abort,
    },
    valid: !enabled || isolationReadOnlyRootsAreAbsolute(parsedRoots),
    update,
  };
}
