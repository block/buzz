import { normalizeFromHandle } from "@/features/search/lib/parseSearchOperators";
import {
  containsNsecShapedInput,
  parsePubkeyInput,
} from "@/shared/lib/nostrUtils";

/** Whether a desktop search string may be sent to a relay-backed query. */
export function isRelaySearchInputSafe(input: string): boolean {
  return !containsNsecShapedInput(input);
}

/**
 * Normalize a user-entered relay search without retaining secret-shaped text
 * in a request or React Query cache key. The `safe` bit stays separate from
 * the redacted query so `allowEmpty` callers cannot accidentally turn an nsec
 * into an enabled empty search.
 */
export function prepareRelaySearchInput(input: string): {
  normalizedQuery: string;
  safe: boolean;
} {
  const normalizedQuery = input.trim().toLowerCase();
  if (!isRelaySearchInputSafe(normalizedQuery)) {
    return { normalizedQuery: "", safe: false };
  }
  return { normalizedQuery, safe: true };
}

export function shouldEnableRelaySearch({
  enabled,
  hasSearchQuery,
  input,
}: {
  enabled: boolean;
  hasSearchQuery: boolean;
  input: string;
}): boolean {
  return enabled && hasSearchQuery && isRelaySearchInputSafe(input);
}

/**
 * Build the optional relay user-lookup query for a `from:` operator.
 * Public keys resolve directly; nsec-shaped values must never leave-device.
 */
export function getFromHandleLookupQuery(raw: string | null): string {
  if (!raw || !isRelaySearchInputSafe(raw) || parsePubkeyInput(raw)) {
    return "";
  }
  return normalizeFromHandle(raw);
}
