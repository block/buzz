import * as React from "react";
import type { FilesystemIsolationProfile } from "@/shared/api/types";
import type { ManagedAgent } from "@/shared/api/types";
import { isMacPlatform } from "@/shared/lib/platform";

export type FilesystemIsolationFieldProps = {
  available: boolean;
  enabled: boolean;
  readOnlyRoots: string;
  onEnabledChange: (value: boolean) => void;
  onReadOnlyRootsChange: (value: string) => void;
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

  // Polling refreshes must not wipe an in-progress edit.
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset only on open or identity switch
  React.useEffect(() => {
    if (!open) return;
    setEnabled(agent.filesystemIsolation?.mode === "ephemeral");
    setReadOnlyRoots(agent.filesystemIsolation?.readOnlyRoots.join("\n") ?? "");
  }, [open, agent.pubkey]);

  const parsedRoots = parseIsolationReadOnlyRoots(readOnlyRoots);

  return {
    fieldProps: {
      available: filesystemIsolationIsAvailable(
        agent.backend.type,
        isMacPlatform(),
      ),
      enabled,
      readOnlyRoots,
      onEnabledChange: setEnabled,
      onReadOnlyRootsChange: setReadOnlyRoots,
    },
    valid: !enabled || isolationReadOnlyRootsAreAbsolute(parsedRoots),
    update: resolveFilesystemIsolationUpdate(
      enabled,
      readOnlyRoots,
      agent.filesystemIsolation,
    ),
  };
}
