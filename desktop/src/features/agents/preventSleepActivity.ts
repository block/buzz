type ObserverEventSummary = {
  seq: number;
  timestamp: string;
};

type AgentObserverEvents = {
  pubkey: string;
  events: readonly ObserverEventSummary[];
};

function latestEventKey(
  events: readonly ObserverEventSummary[],
): string | null {
  const latest = events.length > 0 ? events[events.length - 1] : undefined;
  return latest ? `${latest.timestamp}:${latest.seq}` : null;
}

export function createPreventSleepActivityTracker() {
  const lastEventByAgent = new Map<string, string>();

  return {
    observe(agents: readonly AgentObserverEvents[]): boolean {
      let hasNewActivity = false;

      for (const agent of agents) {
        const latestKey = latestEventKey(agent.events);
        if (!latestKey) continue;

        const previousKey = lastEventByAgent.get(agent.pubkey);
        lastEventByAgent.set(agent.pubkey, latestKey);

        if (previousKey !== undefined && previousKey !== latestKey) {
          hasNewActivity = true;
        }
      }

      return hasNewActivity;
    },
  };
}
