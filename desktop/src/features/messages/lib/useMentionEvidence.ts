import * as React from "react";

/** A bounded verification window, not a directory polling loop. */
export function useMentionEvidence({
  scope,
  request,
  agentKeys,
  directoryUpdatedAt,
  directoryError,
  retry,
}: {
  scope: string;
  request: object | null;
  agentKeys: ReadonlySet<string>;
  directoryUpdatedAt: number;
  directoryError: boolean;
  retry: () => void | Promise<void>;
}) {
  const known = React.useRef({ scope, keys: new Set<string>() });
  if (known.current.scope !== scope) known.current = { scope, keys: new Set() };
  for (const key of agentKeys) known.current.keys.add(key);
  const generation = React.useRef({ scope, token: 0 });
  if (generation.current.scope !== scope) {
    generation.current = { scope, token: generation.current.token + 1 };
  }
  const [retryState, setRetryState] = React.useState<{
    scope: string;
    pending: boolean;
    failed: boolean;
  } | null>(null);
  React.useEffect(
    () => () => {
      generation.current.token += 1;
    },
    [],
  );
  const [attempt, setAttempt] = React.useState(0);
  const [expired, setExpired] = React.useState<{
    request: object;
    attempt: number;
  } | null>(null);
  const [now, setNow] = React.useState(Date.now);
  React.useEffect(() => {
    if (!request || !scope) return;
    const timer = setTimeout(() => setExpired({ request, attempt }), 5000);
    return () => clearTimeout(timer);
  }, [scope, request, attempt]);
  React.useEffect(() => {
    setNow(Date.now());
    const delay = directoryUpdatedAt + 180_000 - Date.now();
    if (delay <= 0) return;
    const timer = setTimeout(() => setNow(Date.now()), delay);
    return () => clearTimeout(timer);
  }, [directoryUpdatedAt]);
  const retryVerification = React.useCallback(() => {
    const token = ++generation.current.token;
    setRetryState({ scope, pending: true, failed: false });
    setExpired(null);
    setAttempt((value) => value + 1);
    void Promise.resolve()
      .then(retry)
      .then(
        () => {
          if (
            generation.current.scope !== scope ||
            generation.current.token !== token
          )
            return;
          setRetryState({ scope, pending: false, failed: false });
          setExpired(null);
          setAttempt((value) => value + 1);
        },
        () => {
          if (
            generation.current.scope !== scope ||
            generation.current.token !== token
          )
            return;
          setRetryState({ scope, pending: false, failed: true });
        },
      );
  }, [retry, scope]);
  return {
    knownAgentPubkeys: known.current.keys,
    verificationPending: retryState?.scope === scope && retryState.pending,
    verificationFailed:
      (retryState?.scope === scope && retryState.failed) ||
      directoryError ||
      (!!request &&
        expired?.request === request &&
        expired.attempt === attempt),
    presenceFresh: directoryUpdatedAt > 0 && now - directoryUpdatedAt < 180_000,
    retryVerification,
  };
}
