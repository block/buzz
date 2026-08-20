import type { EnvVarsValue } from "./EnvVarsEditor";

/**
 * Provider selection changes which values a consumer reads. It must not delete
 * values from the shared env map: global credentials and endpoints may still
 * belong to another explicitly configured agent or to card minting.
 */
export function envVarsPreservingProviderState(
  current: EnvVarsValue,
  _previousProvider: string,
  _nextProvider: string,
): EnvVarsValue {
  return current;
}
