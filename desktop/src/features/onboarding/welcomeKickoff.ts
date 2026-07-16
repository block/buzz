import * as React from "react";

import {
  managedAgentsQueryKey,
  useAcpRuntimesQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { welcomeKickoffMarker } from "@/features/onboarding/devFreshOnboarding";
import { resolveAgentReadiness } from "@/features/onboarding/ui/agentReadiness";
import {
  ensureWelcomeTeam,
  pickWelcomeTeamStarterAgentForRelay,
  WELCOME_TEAM_STARTERS,
  type WelcomeTeamStarterDefinition,
} from "@/features/onboarding/welcomeGuide";
import { isWelcomeChannel } from "@/features/onboarding/welcome";
import { startManagedAgent } from "@/shared/api/tauriManagedAgents";
import { hasManagedAgentChannelMessageMarker } from "@/shared/api/tauriManagedAgentMessageMarkers";
import { sendManagedAgentChannelMessage } from "@/shared/api/tauriManagedAgentMessages";
import type { Channel, ManagedAgent, RelayEvent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useQueryClient } from "@tanstack/react-query";

export const WELCOME_KICKOFF_OPENER_MARKER = "buzz-welcome-kickoff.opener.v1";
export const WELCOME_KICKOFF_CLOSER_MARKER = "buzz-welcome-kickoff.closer.v1";
export const WELCOME_KICKOFF_PROVIDER_MARKER =
  "buzz-welcome-kickoff.provider-required.v1";

const openerMarker = welcomeKickoffMarker(WELCOME_KICKOFF_OPENER_MARKER);
const closerMarker = welcomeKickoffMarker(WELCOME_KICKOFF_CLOSER_MARKER);
const providerMarker = welcomeKickoffMarker(WELCOME_KICKOFF_PROVIDER_MARKER);

export const WELCOME_KICKOFF_PROVIDER_MESSAGE =
  "To get started with agents, connect to an AI provider in Settings. Once you're connected, come back here and we'll introduce the team.";

const WELCOME_KICKOFF_CTA =
  "What can we help you build? Bring us something you're working on, or give us a quick challenge to see how we work together.";
const kickoffInFlight = new Set<string>();
const closerInFlight = new Set<string>();

type WelcomeAgentSet = {
  lead: ManagedAgent;
  teammates: [ManagedAgent, ManagedAgent];
};

function markerEvent(events: readonly RelayEvent[], marker: string) {
  return events.find((event) =>
    event.tags.some(
      (tag) => tag.length >= 2 && tag[0] === "client" && tag[1] === marker,
    ),
  );
}

export function resolveWelcomeAgentSet(
  agents: readonly ManagedAgent[],
): WelcomeAgentSet | null {
  const ordered = WELCOME_TEAM_STARTERS.map((starter) =>
    pickWelcomeTeamStarterAgentForRelay([...agents], starter),
  );
  if (ordered.some((agent) => !agent)) return null;
  return {
    lead: ordered[0] as ManagedAgent,
    teammates: [ordered[1] as ManagedAgent, ordered[2] as ManagedAgent],
  };
}

export function buildWelcomeKickoffOpener(
  lead: ManagedAgent,
  teammates: readonly [ManagedAgent, ManagedAgent],
) {
  return `Hi, I'm ${lead.name}. Welcome to Buzz. This is your private home base, and we're here to help you get oriented or work through something you're building.\n\n@${teammates[0].name} and @${teammates[1].name}, introduce yourselves in a sentence or two — share what you're good at and when to bring you in. Don't start any work yet.`;
}

export function areWelcomeTeammatesOnline(
  teammates: readonly ManagedAgent[],
  presence: Readonly<Record<string, string>> | undefined,
) {
  return teammates.every(
    (agent) => presence?.[normalizePubkey(agent.pubkey)] === "online",
  );
}

export function buildWelcomeKickoffCloser(failedNames: readonly string[]) {
  if (failedNames.length === 0) return WELCOME_KICKOFF_CTA;
  if (failedNames.length === 1) {
    return `${failedNames[0]} is having trouble starting — you can check on them in Agents.\n\n${WELCOME_KICKOFF_CTA}`;
  }
  return `${failedNames.join(" and ")} couldn't start. You can check on them in Agents; I'm still here to help.\n\n${WELCOME_KICKOFF_CTA}`;
}

function introAuthorsAfterOpener(
  events: readonly RelayEvent[],
  opener: RelayEvent,
  teammates: readonly [ManagedAgent, ManagedAgent],
) {
  const authors = new Set(
    events
      .filter((event) => event.created_at >= opener.created_at)
      .map((event) => normalizePubkey(event.pubkey)),
  );
  return new Set(
    teammates
      .filter((agent) => authors.has(normalizePubkey(agent.pubkey)))
      .map((agent) => normalizePubkey(agent.pubkey)),
  );
}

function failedAfterKickoff(agent: ManagedAgent, opener: RelayEvent) {
  if (agent.status !== "stopped" || !agent.lastError || !agent.lastStoppedAt) {
    return false;
  }
  return (
    Math.floor(new Date(agent.lastStoppedAt).getTime() / 1_000) >=
    opener.created_at
  );
}

async function markerExists(channelId: string, marker: string) {
  return hasManagedAgentChannelMessageMarker({
    channelId,
    marker,
    markerScope: "channel",
  });
}

/** Runs the Welcome choreography only while the Welcome channel is focused. */
export function useWelcomeKickoff(
  activeChannel: Channel | null,
  channelEvents: readonly RelayEvent[],
) {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const runtimesQuery = useAcpRuntimesQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const { globalConfig, isLoading: configLoading } = useGlobalAgentConfig();
  const channelId = activeChannel?.id ?? null;
  const isActiveWelcome = isWelcomeChannel(activeChannel);
  const agentSet = React.useMemo(() => {
    const relayUrl = activeCommunity?.relayUrl?.trim().replace(/\/+$/, "");
    return resolveWelcomeAgentSet(
      (managedAgentsQuery.data ?? []).filter(
        (agent) =>
          !relayUrl || agent.relayUrl?.trim().replace(/\/+$/, "") === relayUrl,
      ),
    );
  }, [activeCommunity?.relayUrl, managedAgentsQuery.data]);
  const readiness = React.useMemo(
    () => resolveAgentReadiness(runtimesQuery.data ?? [], globalConfig),
    [globalConfig, runtimesQuery.data],
  );
  const teammatePubkeys = React.useMemo(
    () => agentSet?.teammates.map((agent) => agent.pubkey) ?? [],
    [agentSet],
  );
  const teammatePresence = usePresenceQuery(teammatePubkeys, {
    enabled: isActiveWelcome,
  });

  React.useEffect(() => {
    if (
      !channelId ||
      !isActiveWelcome ||
      configLoading ||
      runtimesQuery.isPending ||
      kickoffInFlight.has(channelId)
    ) {
      return;
    }

    kickoffInFlight.add(channelId);
    void (async () => {
      try {
        const resolvedAgentSet = agentSet;
        if (!resolvedAgentSet) {
          await ensureWelcomeTeam(channelId, activeCommunity?.relayUrl);
          await queryClient.invalidateQueries({
            queryKey: managedAgentsQueryKey,
          });
          return;
        }

        if (await markerExists(channelId, closerMarker)) {
          return;
        }
        if (!readiness.ready) {
          await sendManagedAgentChannelMessage({
            agentPubkey: resolvedAgentSet.lead.pubkey,
            channelId,
            content: WELCOME_KICKOFF_PROVIDER_MESSAGE,
            marker: providerMarker,
            markerScope: "channel",
          });
          return;
        }
        const openerAlreadySent = await markerExists(channelId, openerMarker);

        // Start before publishing the mention. buzz-acp replays events from its
        // startup watermark, so no separate subscription-ready wait is needed.
        // On resume, restart unresolved teammates but never replay the opener.
        const agentsToStart = openerAlreadySent
          ? resolvedAgentSet.teammates
          : [resolvedAgentSet.lead, ...resolvedAgentSet.teammates];
        const startResults = await Promise.allSettled(
          agentsToStart.map((agent) =>
            agent.status === "running" || agent.status === "deployed"
              ? Promise.resolve(agent)
              : startManagedAgent(agent.pubkey),
          ),
        );
        for (const [index, result] of startResults.entries()) {
          if (result.status === "rejected") {
            console.warn(
              `Failed to start Welcome teammate ${agentsToStart[index]?.name ?? "unknown"}.`,
              result.reason,
            );
          }
        }
        await queryClient.invalidateQueries({
          queryKey: managedAgentsQueryKey,
        });
        if (openerAlreadySent) return;

        const allTeammatesReady = areWelcomeTeammatesOnline(
          resolvedAgentSet.teammates,
          teammatePresence.data,
        );
        if (!allTeammatesReady) {
          await teammatePresence.refetch();
          return;
        }

        await sendManagedAgentChannelMessage({
          agentPubkey: resolvedAgentSet.lead.pubkey,
          channelId,
          content: buildWelcomeKickoffOpener(
            resolvedAgentSet.lead,
            resolvedAgentSet.teammates,
          ),
          marker: openerMarker,
          markerScope: "channel",
          mentionPubkeys: resolvedAgentSet.teammates.map(
            (agent) => agent.pubkey,
          ),
        });
      } catch (error) {
        console.warn("Failed to start the Welcome team kickoff.", error);
      } finally {
        kickoffInFlight.delete(channelId);
      }
    })();
  }, [
    activeCommunity?.relayUrl,
    agentSet,
    channelId,
    configLoading,
    isActiveWelcome,
    queryClient,
    readiness,
    runtimesQuery.isPending,
    teammatePresence.data,
    teammatePresence.refetch,
  ]);

  React.useEffect(() => {
    if (
      !channelId ||
      !isActiveWelcome ||
      !agentSet ||
      closerInFlight.has(channelId)
    )
      return;
    const opener = markerEvent(channelEvents, openerMarker);
    if (!opener || markerEvent(channelEvents, closerMarker)) {
      return;
    }

    const introAuthors = introAuthorsAfterOpener(
      channelEvents,
      opener,
      agentSet.teammates,
    );
    const failed = agentSet.teammates.filter((agent) =>
      failedAfterKickoff(agent, opener),
    );
    const resolvedCount = agentSet.teammates.filter(
      (agent) =>
        introAuthors.has(normalizePubkey(agent.pubkey)) ||
        failed.includes(agent),
    ).length;
    if (resolvedCount !== agentSet.teammates.length) return;

    closerInFlight.add(channelId);
    void sendManagedAgentChannelMessage({
      agentPubkey: agentSet.lead.pubkey,
      channelId,
      content: buildWelcomeKickoffCloser(failed.map((agent) => agent.name)),
      marker: closerMarker,
      markerScope: "channel",
    })
      .catch((error) => {
        console.warn("Failed to finish the Welcome team kickoff.", error);
      })
      .finally(() => closerInFlight.delete(channelId));
  }, [agentSet, channelEvents, channelId, isActiveWelcome]);
}

export type { WelcomeTeamStarterDefinition };
