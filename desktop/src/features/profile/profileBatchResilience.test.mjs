import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("useUsersBatchQuery: retry/backoff resilience overrides are present", async () => {
  const source = await readFile(new URL("./hooks.ts", import.meta.url), "utf8");

  // Scoped retry override (not the global default of 1) must be present in
  // the useUsersBatchQuery useQuery call. A cold channel fetch that exhausts
  // retry: 1 would leave raw npubs permanently; retry: 3 gives 3 attempts
  // before the caller needs to kick the channel.
  assert.match(
    source,
    /retry:\s*3/,
    "useUsersBatchQuery must override retry to 3",
  );

  // Exponential backoff capped at 30s must be present.
  assert.match(
    source,
    /retryDelay/,
    "useUsersBatchQuery must specify retryDelay",
  );

  // refetchOnWindowFocus must be set to the error-only predicate: unconditional
  // focus-refetch fires on every Playwright focus event and breaks the
  // profile-hover E2E smoke test; the error-gated form restores post-exhaustion
  // auto-heal without that side-effect.
  assert.match(
    source,
    /refetchOnWindowFocus:\s*\(query\)\s*=>/,
    "useUsersBatchQuery must use error-gated refetchOnWindowFocus, not unconditional true or omit it",
  );
  assert.doesNotMatch(
    source,
    /refetchOnWindowFocus:\s*true/,
    "useUsersBatchQuery must not override refetchOnWindowFocus to unconditional true (breaks E2E hover tests)",
  );
});

test("useUsersBatchQuery: global queryClient defaults are unchanged", async () => {
  const source = await readFile(
    new URL("../../shared/api/queryClient.ts", import.meta.url),
    "utf8",
  );
  // Global retry budget must remain 1 — other queries chose that semantics.
  // Query-deadline failures are exempt so a slow server query is not re-run.
  assert.match(
    source,
    /failureCount < 1 && !isQueryDeadlineError\(error\)/,
    "global retry default must remain 1, skipping query deadlines",
  );
  // Global refetchOnWindowFocus must remain false.
  assert.match(
    source,
    /refetchOnWindowFocus:\s*false/,
    "global refetchOnWindowFocus must remain false",
  );
});
