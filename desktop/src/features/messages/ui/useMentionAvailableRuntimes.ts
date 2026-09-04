import * as React from "react";
import { useAvailableAcpRuntimes } from "@/features/agents/hooks";
import type { AcpRuntime } from "@/shared/api/types";

/** Resolve available runtimes for persona mentions without refetching warm data. */
export function useMentionAvailableRuntimes() {
  const availableRuntimesQuery = useAvailableAcpRuntimes();
  const getAvailableRuntimes = React.useCallback(async (): Promise<
    AcpRuntime[]
  > => {
    const cached = availableRuntimesQuery.data ?? [];
    if (cached.length > 0 || !availableRuntimesQuery.isLoading) {
      return cached;
    }
    const refetched = await availableRuntimesQuery.refetch();
    return (refetched.data ?? []).filter(
      (runtime): runtime is AcpRuntime =>
        runtime.availability === "available" &&
        runtime.command !== null &&
        runtime.binaryPath !== null,
    );
  }, [
    availableRuntimesQuery.data,
    availableRuntimesQuery.isLoading,
    availableRuntimesQuery.refetch,
  ]);
  return getAvailableRuntimes;
}
