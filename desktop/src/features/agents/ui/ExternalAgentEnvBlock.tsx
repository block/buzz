import { AlertTriangle } from "lucide-react";
import * as React from "react";

import { getExternalAgentEnv } from "@/shared/api/tauriAgentBackends";

import { CopyButton } from "./CopyButton";

/**
 * Reveal-and-copy for an external agent's container env.
 *
 * Modeled on `NsecRevealRow` in `features/settings/ui/ProfileSettingsCard`:
 * plain `useState` (never React Query — the block contains the agent's nsec and
 * must not sit in the query cache), fetched only on expand, cleared on collapse
 * and unmount, with a cancellation ref so a late-resolving fetch cannot
 * repopulate state after the user hid it.
 *
 * Re-revealable by design: the user rebuilds the container, so a create-time
 * one-shot would not be enough.
 */
export function ExternalAgentEnvBlock({
  agentName,
  pubkey,
}: {
  agentName?: string;
  pubkey: string;
}) {
  const [isOpen, setIsOpen] = React.useState(false);
  const [envFile, setEnvFile] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(false);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  // Guards against a late-resolving fetch repopulating state after Hide or
  // after the dialog unmounts.
  const fetchCancelledRef = React.useRef(false);

  React.useEffect(() => {
    return () => {
      fetchCancelledRef.current = true;
      setEnvFile(null);
    };
  }, []);

  async function handleToggle() {
    if (isOpen) {
      // Cancel any in-flight fetch and reset every piece of its state. The
      // `finally` below is gated on this same ref, so it will not run for the
      // cancelled fetch — without clearing here, `isLoading` would stay true
      // forever. Not reachable through the toggle today (it is disabled while
      // loading), but that makes correctness depend on a button's disabled
      // state, which is not an invariant worth relying on.
      fetchCancelledRef.current = true;
      setEnvFile(null);
      setIsLoading(false);
      setLoadError(null);
      setIsOpen(false);
      return;
    }
    fetchCancelledRef.current = false;
    setIsOpen(true);
    setIsLoading(true);
    setLoadError(null);
    try {
      const result = await getExternalAgentEnv(pubkey);
      if (!fetchCancelledRef.current) setEnvFile(result.envFile);
    } catch (error) {
      if (!fetchCancelledRef.current)
        setLoadError(
          error instanceof Error
            ? error.message
            : "Failed to build the env block.",
        );
    } finally {
      if (!fetchCancelledRef.current) setIsLoading(false);
    }
  }

  return (
    <div
      className="rounded-2xl border border-border/70 bg-muted/20 p-4"
      data-testid="external-agent-env-block"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-sm font-semibold tracking-tight">
            Container environment
          </p>
          <p className="text-sm text-muted-foreground">
            Everything <span className="font-mono">buzz-acp</span> needs to run
            {agentName ? ` ${agentName}` : " this agent"} wherever it lives.
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {isOpen && envFile ? (
            <CopyButton label="Copy env" value={envFile} />
          ) : null}
          <button
            className="text-sm font-medium text-primary hover:underline"
            disabled={isLoading}
            onClick={handleToggle}
            type="button"
          >
            {isOpen ? "Hide" : isLoading ? "Loading…" : "Show"}
          </button>
        </div>
      </div>

      {isOpen ? (
        <div className="mt-3 space-y-3">
          <div className="flex gap-3 rounded-xl border border-warning/30 bg-warning-bg px-3 py-2">
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" />
            <p className="text-sm text-warning">
              Contains the agent&apos;s private key. Prefer writing it to a file
              and passing <span className="font-mono">--env-file</span> — values
              given with <span className="font-mono">-e</span> land in your
              shell history and in{" "}
              <span className="font-mono">docker inspect</span>.
            </p>
          </div>

          {loadError ? (
            <p className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {loadError}
            </p>
          ) : envFile ? (
            <>
              <pre className="max-h-64 overflow-auto rounded-xl border border-border/70 bg-background/80 px-3 py-2 text-xs">
                <code>{envFile}</code>
              </pre>
              <p className="text-sm text-muted-foreground">
                The image needs <span className="font-mono">buzz-acp</span>, the
                agent binary named by{" "}
                <span className="font-mono">BUZZ_ACP_AGENT_COMMAND</span>, and
                the <span className="font-mono">buzz</span> CLI. Changing this
                agent&apos;s settings later does not reach a running container —
                come back here and re-copy.
              </p>
            </>
          ) : (
            <p className="text-sm text-muted-foreground">Loading…</p>
          )}
        </div>
      ) : null}
    </div>
  );
}
