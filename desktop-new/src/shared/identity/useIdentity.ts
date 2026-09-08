import { useCallback, useState } from "react";
import { runtime } from "@/shared/runtime/client";
import type { Identity } from "@/features/sessions/types";

/**
 * Who the person using this client is.
 *
 * A fact, not a capability: it answers a question and owns no policy. Nothing in
 * the app can legitimately disagree about it, so it has one reader-facing shape
 * and no fallback, retry, or ordering rules of its own.
 *
 * Deliberately not cached at module level. A cache here would be
 * community-scoped state owing a teardown, which is a real obligation this fact
 * does not need — the backend call is cheap and the value is stable.
 */
export async function readIdentity(): Promise<Identity> {
  return runtime.identity();
}

/** The same fact for a component that wants it in render. */
export function useIdentity() {
  const [identity, setIdentity] = useState<Identity | null>(null);

  const load = useCallback(async () => {
    const next = await readIdentity();
    setIdentity(next);
    return next;
  }, []);

  return { identity, load };
}
