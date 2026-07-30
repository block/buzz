import * as React from "react";

import {
  readNumbatFindings,
  type NumbatFinding,
} from "@/shared/api/tauriNumbat";

const POLL_INTERVAL_MS = 2_000;
const MAX_FINDINGS = 100;

export function useNumbatFindings(
  agentPubkey: string,
  channelId: string | null,
  sessionId: string | null,
  turnId: string | null,
) {
  const [findings, setFindings] = React.useState<NumbatFinding[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const [health, setHealth] = React.useState<{
    state: "configured" | "disconnected" | "unsupported" | "stale";
    detail: string;
  } | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    let offset = 0;
    let timeoutId: number | null = null;
    setFindings([]);
    setError(null);
    setHealth(null);

    async function poll() {
      try {
        const batch = await readNumbatFindings(
          agentPubkey,
          offset,
          sessionId,
          channelId,
          turnId,
        );
        if (cancelled) return;

        offset = batch.nextOffset;
        setHealth(batch.health);
        setError(null);
        setFindings((current) => {
          const base = batch.reset ? [] : current;
          const byId = new Map(
            base.map((finding) => [finding.findingId, finding]),
          );
          for (const finding of batch.findings) {
            byId.set(finding.findingId, finding);
          }
          return [...byId.values()]
            .sort((left, right) =>
              left.detectedAt.localeCompare(right.detectedAt),
            )
            .slice(-MAX_FINDINGS);
        });
      } catch (cause) {
        if (!cancelled) {
          setHealth({
            state: "stale",
            detail: "Guardian telemetry is temporarily unavailable.",
          });
          setError(
            cause instanceof Error
              ? cause.message
              : "Could not read local security findings.",
          );
        }
      } finally {
        if (!cancelled) {
          timeoutId = window.setTimeout(poll, POLL_INTERVAL_MS);
        }
      }
    }

    void poll();
    return () => {
      cancelled = true;
      if (timeoutId !== null) window.clearTimeout(timeoutId);
    };
  }, [agentPubkey, channelId, sessionId, turnId]);

  const scopedFindings = React.useMemo(
    () =>
      findings.filter((finding) =>
        channelId === null
          ? finding.channelId === null
          : finding.channelId === channelId,
      ),
    [channelId, findings],
  );

  return { error, findings: scopedFindings, health };
}
