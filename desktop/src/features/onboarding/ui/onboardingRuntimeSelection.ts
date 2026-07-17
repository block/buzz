import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

export function runtimeCanBeSelected(runtime: AcpRuntimeCatalogEntry) {
  if (runtime.availability !== "available") return false;
  if (runtime.id === "claude" || runtime.id === "codex") {
    return (
      runtime.authStatus.status === "logged_in" ||
      runtime.authStatus.status === "not_applicable"
    );
  }
  return runtime.id === "buzz-agent" || runtime.id === "goose";
}
