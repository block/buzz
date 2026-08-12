import type { CodexSharedRuntimeStatus } from "@/shared/api/codexTaskTypes";

export function hasCodexDesktopRuntimeConflict(
  status: CodexSharedRuntimeStatus | null | undefined,
): boolean {
  return (status?.privateAppServerProcessIds.length ?? 0) > 0;
}

export function isCodexSharedRuntimeUsable(
  status: CodexSharedRuntimeStatus | null | undefined,
): boolean {
  return (
    status?.state === "ready" &&
    !status.desktopDetectionError &&
    !hasCodexDesktopRuntimeConflict(status)
  );
}
