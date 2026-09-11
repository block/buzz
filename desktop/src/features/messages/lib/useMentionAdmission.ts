import * as React from "react";
import { AgentMentionAuthorizationError } from "./agentMentionRevalidation";

/** One cancellable prepare/commit owner. No caller may mutate while preparing. */
export function useMentionAdmission(scope: object) {
  const activeKey = React.useRef<object | null>(null);
  const activeValid = React.useRef<(() => boolean) | null>(null);
  const generation = React.useRef(0);
  const timer = React.useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const releaseNavigation = React.useRef<(() => void) | undefined>(undefined);
  const [status, setStatus] = React.useState("");
  const cancel = React.useCallback(() => {
    releaseNavigation.current?.();
    releaseNavigation.current = undefined;
    generation.current += 1;
    activeKey.current = null;
    activeValid.current = null;
    clearTimeout(timer.current);
    setStatus("");
  }, []);
  // Losing live eligibility abandons this operation, even if Retry later
  // restores the same row before its older prepare promise settles.
  React.useLayoutEffect(() => {
    if (activeValid.current && !activeValid.current()) cancel();
  });
  // biome-ignore lint/correctness/useExhaustiveDependencies: scope changes abandon the operation even when the value returns later.
  React.useLayoutEffect(() => {
    cancel();
    return () => {
      releaseNavigation.current?.();
      releaseNavigation.current = undefined;
      generation.current += 1;
      clearTimeout(timer.current);
    };
  }, [scope, cancel]);
  const begin = React.useCallback(
    (operation: {
      key: object;
      valid: () => boolean;
      prepare: () => Promise<unknown>;
      commit: () => void;
    }) => {
      if (activeKey.current === operation.key) return;
      cancel();
      if (!operation.valid()) return;
      activeKey.current = operation.key;
      activeValid.current = operation.valid;
      // Admission belongs to the focused action, not the composer's wider
      // focus ownership. Observe its departure even inside an overlay/portal;
      // returning later must not resurrect the pending operation.
      const origin = document.activeElement;
      const view = origin?.ownerDocument.defaultView;
      origin?.addEventListener("blur", cancel);
      view?.addEventListener("blur", cancel);
      releaseNavigation.current = () => {
        origin?.removeEventListener("blur", cancel);
        view?.removeEventListener("blur", cancel);
      };
      const id = generation.current;
      const current = () => id === generation.current && operation.valid();
      setStatus("Checking access…");
      timer.current = setTimeout(() => {
        if (id !== generation.current) return;
        const valid = operation.valid();
        cancel();
        if (valid) setStatus("Could not check access. Select again to retry.");
      }, 15000);
      void (async () => {
        let committing = false;
        try {
          await operation.prepare();
          if (!current()) return;
          clearTimeout(timer.current);
          setStatus("");
          // No await between the final fence and the complete consumer commit.
          releaseNavigation.current?.();
          releaseNavigation.current = undefined;
          committing = true;
          operation.commit();
        } catch (error) {
          if (committing) {
            console.error("Mention selection commit failed", error);
            setStatus(
              "Could not finish selection. Check the draft before retrying.",
            );
            return;
          }
          if (!current()) return;
          setStatus(
            error instanceof AgentMentionAuthorizationError &&
              error.reason === "denied"
              ? "Access changed. Selection was not inserted. Select again to retry."
              : "Could not check access. Select again to retry.",
          );
        } finally {
          if (id === generation.current) {
            releaseNavigation.current?.();
            releaseNavigation.current = undefined;
            clearTimeout(timer.current);
            activeKey.current = null;
            activeValid.current = null;
            if (!committing && !operation.valid()) setStatus("");
          }
        }
      })();
    },
    [cancel],
  );
  return { begin, cancel, status };
}
