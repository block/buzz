import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { parsePulseSummary, type buildSummaryInput } from "./lib/pulseSummary";

export function usePulseSummary(
  input: ReturnType<typeof buildSummaryInput>,
  enabled: boolean,
) {
  const serialized = JSON.stringify(input);
  const [snapshot, setSnapshot] = React.useState("");
  const lastAccepted = React.useRef(0);
  React.useEffect(() => {
    if (!enabled || serialized === snapshot) return;
    const timer = setTimeout(
      () => {
        setSnapshot(serialized);
        lastAccepted.current = Date.now();
      },
      Math.max(1200, 60_000 - (Date.now() - lastAccepted.current)),
    );
    return () => clearTimeout(timer);
  }, [serialized, snapshot, enabled]);
  const snapshotInput = snapshot
    ? (JSON.parse(snapshot) as typeof input)
    : null;
  return useQuery({
    queryKey: ["pulse-summary", input.scope, snapshot],
    enabled:
      enabled &&
      Boolean(snapshotInput?.conversations.length) &&
      snapshotInput?.scope === input.scope,
    queryFn: async ({ signal }) => {
      const response = await fetch("/__pulse/briefing", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: snapshot,
        signal,
      });
      if (!response.ok)
        throw new Error("Couldn’t summarize your activity. Try again.");
      return parsePulseSummary(
        await response.json(),
        new Set(
          snapshotInput?.conversations.flatMap((item) => [
            item.id,
            ...item.messages.map((m) => m.conversationId),
          ]),
        ),
      );
    },
    staleTime: 300_000,
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[1] === input.scope ? previous : undefined,
    gcTime: 60_000,
    retry: false,
    refetchOnWindowFocus: false,
  });
}
