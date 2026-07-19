import * as React from "react";

import {
  managedAgentsQueryKey,
  useAcpRuntimesQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { useGlobalAgentConfig } from "@/features/agents/useGlobalAgentConfig";
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
import { getThreadReference } from "@/features/messages/lib/threading";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import {
  startManagedAgent,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import { hasManagedAgentChannelMessageMarker } from "@/shared/api/tauriManagedAgentMessageMarkers";
import { sendManagedAgentChannelMessage } from "@/shared/api/tauriManagedAgentMessages";
import { getPresence, listManagedAgents } from "@/shared/api/tauri";
import { getProfile } from "@/shared/api/tauriProfiles";
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

function formatAgentNames(agents: readonly ManagedAgent[]) {
  if (agents.length === 0) return "";
  if (agents.length === 1) return agents[0]?.name ?? "";
  return `${agents
    .slice(0, -1)
    .map((agent) => agent.name)
    .join(", ")} and ${agents[agents.length - 1]?.name ?? ""}`;
}

function formatMentionNames(agents: readonly ManagedAgent[]) {
  if (agents.length === 0) return "";
  if (agents.length === 1) return `@${agents[0]?.name ?? ""}`;
  return `${agents
    .slice(0, -1)
    .map((agent) => `@${agent.name}`)
    .join(", ")} and @${agents[agents.length - 1]?.name ?? ""}`;
}
export function createWelcomeKickoffCoordinator() {
  const controllers = new Map<string, AbortController>();
  return {
    begin(channelId: string) {
      if (controllers.has(channelId)) return null;
      const controller = new AbortController();
      controllers.set(channelId, controller);
      return controller;
    },
    cancel(channelId: string) {
      controllers.get(channelId)?.abort();
      controllers.delete(channelId);
    },
    finish(channelId: string, controller: AbortController) {
      if (controllers.get(channelId) === controller) {
        controllers.delete(channelId);
      }
    },
  };
}

const kickoffCoordinator = createWelcomeKickoffCoordinator();
const closerInFlight = new Set<string>();
const TEAMMATE_READY_POLL_MS = 250;
const TEAMMATE_READY_WAIT_MS = 60_000;
/**
 * Give-up backstop for teammates that are neither intro'd nor detectably failed
 * — i.e. alive but silent. **Not** an expectation of how fast an intro arrives.
 *
 * It does not gate the happy path: once every teammate is resolved (intro'd, or
 * `failedAfterKickoff`-detected), the closer fires on `CLOSER_BEAT_MS` and this
 * timer is cleared. Raising it costs the normal case nothing — it only delays
 * the moment we give up on a silent teammate.
 *
 * So it must be long enough that "taking longer than expected" is *true* when it
 * fires. `unresolved` means "no intro seen yet", which is ignorance, not a fact;
 * announcing it early states a falsehood, and the closer marker is terminal
 * (`sendWelcomeKickoffCloser`) so nothing ever corrects it. At 15s this
 * routinely beat two cold agents through harness dispatch + a full LLM turn
 * (observed 2026-07-18: intros landed ~60s in, and the false "taking longer"
 * closer had already been stamped final at 15s). A real failure does not wait
 * for this — `failedAfterKickoff` resolves crashed teammates immediately.
 *
 * See docs/welcome-kickoff-silent-failures.md §1.
 */
const TEAMMATE_INTRO_BACKSTOP_MS = 120_000;
const CLOSER_BEAT_MS = 3_000;
const closerAbortControllers = new Map<string, AbortController>();
const closerTimeouts = new Map<
  string,
  ReturnType<typeof globalThis.setTimeout>
>();

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

function normalizeRelayUrl(relayUrl?: string | null) {
  return relayUrl?.trim().replace(/\/+$/, "") ?? null;
}

function resolveWelcomeAgentSetForRelay(
  agents: readonly ManagedAgent[],
  relayUrl?: string | null,
) {
  const normalizedRelayUrl = normalizeRelayUrl(relayUrl);
  return resolveWelcomeAgentSet(
    agents.filter(
      (agent) =>
        !normalizedRelayUrl ||
        normalizeRelayUrl(agent.relayUrl) === normalizedRelayUrl,
    ),
  );
}

export function buildWelcomeKickoffOpener(
  lead: ManagedAgent,
  introTeammates: readonly ManagedAgent[],
  allTeammates: readonly ManagedAgent[] = introTeammates,
  ownerName?: string | null,
) {
  // Greet the new user by name when we know it. Paired with their pubkey in
  // the p tags, the @mention renders as a pill and files the opener into
  // their Inbox mentions feed.
  const trimmedOwnerName = ownerName?.trim();
  const greeting = trimmedOwnerName
    ? `Hi @${trimmedOwnerName}, I'm ${lead.name}.`
    : `Hi, I'm ${lead.name}.`;
  const introNames = formatMentionNames(introTeammates);
  if (introTeammates.length === 0) {
    const teammateNames = formatAgentNames(allTeammates);
    const teammatePhrase = teammateNames ? ` with ${teammateNames}` : "";
    return `${greeting} Welcome to Buzz. This is your private home base, and I'm here${teammatePhrase} to help you get oriented or work through something you're building.\n\n${WELCOME_KICKOFF_CTA}`;
  }

  return `${greeting} Welcome to Buzz. This is your private home base, and we're here to help you get oriented or work through something you're building.\n\n${introNames}, introduce ${introTeammates.length === 1 ? "yourself" : "yourselves"} in a sentence or two — share what you're good at and when to bring you in. Don't start any work yet.`;
}

export function onlineWelcomeTeammates(
  teammates: readonly ManagedAgent[],
  presence: Readonly<Record<string, string>> | undefined,
) {
  return teammates.filter(
    (agent) => presence?.[normalizePubkey(agent.pubkey)] === "online",
  );
}

export function areWelcomeTeammatesOnline(
  teammates: readonly ManagedAgent[],
  presence: Readonly<Record<string, string>> | undefined,
) {
  return (
    onlineWelcomeTeammates(teammates, presence).length === teammates.length
  );
}

export async function waitForWelcomeTeammatesOnline(
  teammates: readonly ManagedAgent[],
  options: {
    isCancelled: () => boolean;
    loadPresence?: typeof getPresence;
    pollMs?: number;
    waitMs?: number;
  },
) {
  const loadPresence = options.loadPresence ?? getPresence;
  const pollMs = options.pollMs ?? TEAMMATE_READY_POLL_MS;
  const deadline = Date.now() + (options.waitMs ?? TEAMMATE_READY_WAIT_MS);
  const pubkeys = teammates.map((agent) => agent.pubkey);
  let latestOnline: ManagedAgent[] = [];

  while (!options.isCancelled()) {
    try {
      latestOnline = onlineWelcomeTeammates(
        teammates,
        await loadPresence(pubkeys),
      );
      if (latestOnline.length === teammates.length) {
        return latestOnline;
      }
    } catch (error) {
      console.warn("Welcome teammate presence check failed; retrying.", error);
    }
    if (Date.now() >= deadline) break;
    await new Promise((resolve) => globalThis.setTimeout(resolve, pollMs));
  }
  return options.isCancelled() ? [] : latestOnline;
}

export async function waitForWelcomeKickoffBeat(options: {
  signal?: AbortSignal;
  waitMs: number;
}) {
  const waitMs = options.waitMs;
  if (options.signal?.aborted) return false;

  return new Promise<boolean>((resolve) => {
    const timer = globalThis.setTimeout(() => {
      options.signal?.removeEventListener("abort", cancel);
      resolve(true);
    }, waitMs);
    const cancel = () => {
      globalThis.clearTimeout(timer);
      resolve(false);
    };
    options.signal?.addEventListener("abort", cancel, { once: true });
  });
}

export function buildWelcomeKickoffCloser(
  failedNames: readonly string[],
  delayedNames: readonly string[] = [],
) {
  if (failedNames.length === 0 && delayedNames.length === 0) {
    return WELCOME_KICKOFF_CTA;
  }
  if (failedNames.length === 1 && delayedNames.length === 0) {
    return `${failedNames[0]} is having trouble starting — you can check on them in Agents.\n\n${WELCOME_KICKOFF_CTA}`;
  }
  if (failedNames.length > 1 && delayedNames.length === 0) {
    return `${failedNames.join(" and ")} couldn't start. You can check on them in Agents; I'm still here to help.\n\n${WELCOME_KICKOFF_CTA}`;
  }
  if (failedNames.length === 0 && delayedNames.length === 1) {
    return `${delayedNames[0]} is taking longer to reply — I'm still here to help.\n\n${WELCOME_KICKOFF_CTA}`;
  }
  const names = [...failedNames, ...delayedNames].join(" and ");
  return `${names} are taking longer than expected. I'm still here to help.\n\n${WELCOME_KICKOFF_CTA}`;
}

function isReplyToOpener(event: RelayEvent, opener: RelayEvent) {
  const threadRef = getThreadReference(event.tags);
  return threadRef.rootId === opener.id || threadRef.parentId === opener.id;
}

function introAuthorsAfterOpener(
  events: readonly RelayEvent[],
  opener: RelayEvent,
  teammates: readonly [ManagedAgent, ManagedAgent],
) {
  const authors = new Set(
    events
      .filter(
        (event) =>
          event.created_at >= opener.created_at &&
          isReplyToOpener(event, opener),
      )
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

export function classifyWelcomeKickoffResolution(
  events: readonly RelayEvent[],
  opener: RelayEvent,
  agentSet: WelcomeAgentSet,
) {
  const introAuthors = introAuthorsAfterOpener(
    events,
    opener,
    agentSet.teammates,
  );
  const failed = agentSet.teammates.filter((agent) =>
    failedAfterKickoff(agent, opener),
  );
  const unresolved = agentSet.teammates.filter(
    (agent) =>
      !introAuthors.has(normalizePubkey(agent.pubkey)) &&
      !failed.includes(agent),
  );
  return { failed, unresolved };
}

async function resolveLatestWelcomeAgentSet({
  fallback,
  queryClient,
  relayUrl,
}: {
  fallback: WelcomeAgentSet;
  queryClient: ReturnType<typeof useQueryClient>;
  relayUrl?: string | null;
}) {
  const agents = await queryClient.fetchQuery({
    queryKey: managedAgentsQueryKey,
    queryFn: listManagedAgents,
  });
  return resolveWelcomeAgentSetForRelay(agents, relayUrl) ?? fallback;
}

async function markerExists(channelId: string, marker: string) {
  return hasManagedAgentChannelMessageMarker({
    channelId,
    marker,
    markerScope: "channel",
  });
}

export function welcomeTeammateNeedsRestart(
  agent: ManagedAgent,
  leadPubkey: string,
) {
  return (
    agent.status === "running" &&
    (agent.needsRestart ||
      agent.respondTo !== "allowlist" ||
      !agent.respondToAllowlist.some(
        (pubkey) => normalizePubkey(pubkey) === normalizePubkey(leadPubkey),
      ))
  );
}

export function selectWelcomeKickoffIntroTeammates(
  teammates: readonly ManagedAgent[],
  onlineTeammates: readonly ManagedAgent[],
) {
  const onlinePubkeys = new Set(
    onlineTeammates.map((agent) => normalizePubkey(agent.pubkey)),
  );
  return teammates.filter((agent) =>
    onlinePubkeys.has(normalizePubkey(agent.pubkey)),
  );
}

export type WelcomeKickoffOwner = {
  pubkey: string;
  displayName?: string | null;
};

/**
 * The event view the kickoff classification reasons over: the channel's own
 * events plus the opener's thread replies.
 *
 * Teammate intros (and the closer) are thread replies, which the channel
 * window deliberately excludes — only broadcast replies reach the main
 * timeline. So `channelEvents` alone shows the opener and never the intros,
 * and the closer stalls until the user happens to open the thread. Merging the
 * opener's subtree in is what lets the choreography resolve on its own.
 *
 * De-duplicated because the two sources legitimately overlap: the live
 * subscription writes replies into the thread cache, and an open thread also
 * feeds the same replies in through `channelEvents`.
 */
export function mergeKickoffEvents(
  channelEvents: readonly RelayEvent[],
  openerReplies: readonly RelayEvent[],
): readonly RelayEvent[] {
  if (openerReplies.length === 0) return channelEvents;
  const seen = new Set(channelEvents.map((event) => event.id));
  return [
    ...channelEvents,
    ...openerReplies.filter((event) => !seen.has(event.id)),
  ];
}

export function buildWelcomeKickoffOpenerSendInput(
  agentSet: WelcomeAgentSet,
  introTeammates: readonly ManagedAgent[],
  channelId: string,
  owner?: WelcomeKickoffOwner | null,
) {
  // Greet the new user by name and tag their pubkey. The p tag renders the
  // "@Name" in the copy as a mention pill and files the opener into their
  // Inbox mentions feed, so the Inbox isn't an empty state on first visit.
  const mentionPubkeys = introTeammates.map((agent) => agent.pubkey);
  if (
    owner?.pubkey &&
    !mentionPubkeys.some(
      (pubkey) => normalizePubkey(pubkey) === normalizePubkey(owner.pubkey),
    )
  ) {
    mentionPubkeys.push(owner.pubkey);
  }
  return {
    agentPubkey: agentSet.lead.pubkey,
    channelId,
    content: buildWelcomeKickoffOpener(
      agentSet.lead,
      introTeammates,
      agentSet.teammates,
      owner?.displayName,
    ),
    marker: openerMarker,
    markerScope: "channel" as const,
    mentionPubkeys,
    additionalMarkers: introTeammates.length === 0 ? [closerMarker] : [],
  };
}

export async function restartWelcomeTeammate(
  agent: ManagedAgent,
  options: {
    stopAgent?: typeof stopManagedAgent;
    startAgent?: typeof startManagedAgent;
  } = {},
) {
  const stopAgent = options.stopAgent ?? stopManagedAgent;
  const startAgent = options.startAgent ?? startManagedAgent;
  if (agent.status === "running") {
    await stopAgent(agent.pubkey);
  }
  return startAgent(agent.pubkey);
}

async function sendWelcomeKickoffCloser({
  agentSet,
  channelId,
  content,
  opener,
}: {
  agentSet: WelcomeAgentSet;
  channelId: string;
  content: string;
  opener: RelayEvent;
}) {
  if (await markerExists(channelId, closerMarker)) return;

  await sendManagedAgentChannelMessage({
    agentPubkey: agentSet.lead.pubkey,
    channelId,
    content,
    marker: closerMarker,
    markerScope: "channel",
    parentEventId: opener.id,
  });
}

/** Runs the Welcome choreography only while the Welcome channel is focused. */
export function useWelcomeKickoff(
  activeChannel: Channel | null,
  channelEvents: readonly RelayEvent[],
  onKickoffOpenerPosted?: (eventId: string) => void,
) {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const runtimesQuery = useAcpRuntimesQuery();
  const managedAgentsQuery = useManagedAgentsQuery();
  const { globalConfig, isLoading: configLoading } = useGlobalAgentConfig();
  const channelId = activeChannel?.id ?? null;
  const isActiveWelcome = isWelcomeChannel(activeChannel);
  const focusedWelcomeChannelRef = React.useRef<string | null>(null);
  focusedWelcomeChannelRef.current = isActiveWelcome ? channelId : null;
  // Watch the opener's thread subtree directly so teammate intro replies are
  // visible to the closer classification even when the user never opens the
  // thread. Without this, replies only surfaced through the UI's open-thread
  // query and the closer stalled until the user clicked into the thread.
  const openerEvent = React.useMemo(
    () => markerEvent(channelEvents, openerMarker) ?? null,
    [channelEvents],
  );
  // Retire the watch once the closer exists: the kickoff is resolved, so
  // revisits to Welcome shouldn't keep refetching the subtree forever.
  //
  // This has to be a latch rather than a plain derivation. The closer is a
  // *thread reply* to the opener (see sendWelcomeKickoffCloser), so it never
  // appears in `channelEvents` unless the user happened to open the thread —
  // deriving from `channelEvents` meant this never retired at all. Deriving
  // from `kickoffEvents` instead is self-referential: it gates the query that
  // feeds it, so retiring would drop the evidence that justified retiring and
  // (on a cache eviction) flip the query back on. Latching per channel keeps
  // the decision one-way and stable.
  const [resolvedChannelId, setResolvedChannelId] = React.useState<
    string | null
  >(null);
  const kickoffResolved = channelId !== null && resolvedChannelId === channelId;
  const openerThreadQuery = useThreadReplies(
    isActiveWelcome && !kickoffResolved ? activeChannel : null,
    openerEvent?.id ?? null,
  );
  const kickoffEvents = React.useMemo(
    () => mergeKickoffEvents(channelEvents, openerThreadQuery.data ?? []),
    [channelEvents, openerThreadQuery.data],
  );
  React.useEffect(() => {
    if (!channelId || kickoffResolved) return;
    if (markerEvent(kickoffEvents, closerMarker) == null) return;
    setResolvedChannelId(channelId);
  }, [channelId, kickoffEvents, kickoffResolved]);
  const channelEventsRef = React.useRef(kickoffEvents);
  channelEventsRef.current = kickoffEvents;
  const agentSet = React.useMemo(
    () =>
      resolveWelcomeAgentSetForRelay(
        managedAgentsQuery.data ?? [],
        activeCommunity?.relayUrl,
      ),
    [activeCommunity?.relayUrl, managedAgentsQuery.data],
  );
  const readiness = React.useMemo(
    () => resolveAgentReadiness(runtimesQuery.data ?? [], globalConfig),
    [globalConfig, runtimesQuery.data],
  );
  React.useEffect(() => {
    if (
      !channelId ||
      !isActiveWelcome ||
      configLoading ||
      runtimesQuery.isPending
    ) {
      return;
    }

    const kickoffController = kickoffCoordinator.begin(channelId);
    if (!kickoffController) return;
    const isCancelled = () =>
      kickoffController.signal.aborted ||
      focusedWelcomeChannelRef.current !== channelId;
    void (async () => {
      try {
        const welcomeTeam = await ensureWelcomeTeam(
          channelId,
          activeCommunity?.relayUrl,
        );
        await queryClient.invalidateQueries({
          queryKey: managedAgentsQueryKey,
        });
        const resolvedAgentSet: WelcomeAgentSet = {
          lead: welcomeTeam[0],
          teammates: [welcomeTeam[1], welcomeTeam[2]],
        };

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
          agentsToStart.map((agent) => {
            const isTeammate = resolvedAgentSet.teammates.some(
              (teammate) =>
                normalizePubkey(teammate.pubkey) ===
                normalizePubkey(agent.pubkey),
            );
            if (
              isTeammate &&
              welcomeTeammateNeedsRestart(agent, resolvedAgentSet.lead.pubkey)
            ) {
              return restartWelcomeTeammate(agent);
            }
            return agent.status === "running" || agent.status === "deployed"
              ? Promise.resolve(agent)
              : startManagedAgent(agent.pubkey);
          }),
        );
        for (const [index, result] of startResults.entries()) {
          if (result.status === "rejected") {
            console.warn(
              `Failed to start Welcome agent ${agentsToStart[index]?.name ?? "unknown"}.`,
              result.reason,
            );
          }
        }
        await queryClient.invalidateQueries({
          queryKey: managedAgentsQueryKey,
        });
        if (openerAlreadySent) return;

        const leadStartIndex = agentsToStart.findIndex(
          (agent) => agent.pubkey === resolvedAgentSet.lead.pubkey,
        );
        if (startResults[leadStartIndex]?.status === "rejected") return;
        const teammatesToAwait = resolvedAgentSet.teammates.filter(
          (teammate) =>
            startResults[
              agentsToStart.findIndex(
                (agent) => agent.pubkey === teammate.pubkey,
              )
            ]?.status !== "rejected",
        );
        const onlineTeammates = await waitForWelcomeTeammatesOnline(
          teammatesToAwait,
          { isCancelled },
        );
        if (isCancelled()) return;
        const introTeammates = selectWelcomeKickoffIntroTeammates(
          resolvedAgentSet.teammates,
          onlineTeammates,
        );
        if (introTeammates.length < resolvedAgentSet.teammates.length) {
          console.warn(
            "Some Welcome teammates did not become ready; continuing with a degraded kickoff.",
          );
        }
        if (isCancelled()) return;

        // Best-effort: a missing profile should degrade to an ungreeted,
        // untagged opener, never block the kickoff.
        const owner = await getProfile()
          .then((profile) => ({
            pubkey: profile.pubkey,
            displayName: profile.displayName,
          }))
          .catch(() => null);
        const openerResult = await sendManagedAgentChannelMessage(
          buildWelcomeKickoffOpenerSendInput(
            resolvedAgentSet,
            introTeammates,
            channelId,
            owner,
          ),
        );
        if (!isCancelled()) onKickoffOpenerPosted?.(openerResult.eventId);
      } catch (error) {
        console.warn("Failed to start the Welcome team kickoff.", error);
      } finally {
        kickoffCoordinator.finish(channelId, kickoffController);
      }
    })();
  }, [
    activeCommunity?.relayUrl,
    channelId,
    configLoading,
    isActiveWelcome,
    onKickoffOpenerPosted,
    queryClient,
    readiness,
    runtimesQuery.isPending,
  ]);

  React.useEffect(() => {
    void isActiveWelcome;
    return () => {
      if (!channelId) return;
      kickoffCoordinator.cancel(channelId);
      closerAbortControllers.get(channelId)?.abort();
      closerAbortControllers.delete(channelId);
      const timeout = closerTimeouts.get(channelId);
      if (timeout) globalThis.clearTimeout(timeout);
      closerTimeouts.delete(channelId);
      closerInFlight.delete(channelId);
    };
  }, [channelId, isActiveWelcome]);

  React.useEffect(() => {
    if (
      !channelId ||
      !isActiveWelcome ||
      !agentSet ||
      closerInFlight.has(channelId) ||
      // Respect the latch, not just the events. Retiring the opener-thread
      // watch drops the subtree from `kickoffEvents`, which is where the closer
      // lives — so once resolved, the marker check below can no longer see it
      // and would classify every teammate as silent and re-run the closer on
      // each revisit. The latch is the durable "already resolved" signal.
      kickoffResolved
    )
      return;
    const opener = markerEvent(kickoffEvents, openerMarker);
    if (!opener || markerEvent(kickoffEvents, closerMarker)) {
      return;
    }

    const { unresolved } = classifyWelcomeKickoffResolution(
      kickoffEvents,
      opener,
      agentSet,
    );

    if (unresolved.length > 0) {
      if (!closerTimeouts.has(channelId)) {
        const elapsedMs = Math.max(0, Date.now() - opener.created_at * 1_000);
        const waitMs = Math.max(0, TEAMMATE_INTRO_BACKSTOP_MS - elapsedMs);
        const timeout = globalThis.setTimeout(() => {
          closerTimeouts.delete(channelId);
          if (focusedWelcomeChannelRef.current !== channelId) return;
          const controller = new AbortController();
          closerAbortControllers.set(channelId, controller);
          closerInFlight.add(channelId);
          void (async () => {
            if (
              !(await waitForWelcomeKickoffBeat({
                signal: controller.signal,
                waitMs: CLOSER_BEAT_MS,
              })) ||
              controller.signal.aborted ||
              focusedWelcomeChannelRef.current !== channelId
            )
              return;

            const latestEvents = channelEventsRef.current;
            if (markerEvent(latestEvents, closerMarker)) return;
            const latestOpener =
              markerEvent(latestEvents, openerMarker) ?? opener;
            const latestAgentSet = await resolveLatestWelcomeAgentSet({
              fallback: agentSet,
              queryClient,
              relayUrl: activeCommunity?.relayUrl,
            });
            const latestResolution = classifyWelcomeKickoffResolution(
              latestEvents,
              latestOpener,
              latestAgentSet,
            );
            await sendWelcomeKickoffCloser({
              agentSet: latestAgentSet,
              channelId,
              content: buildWelcomeKickoffCloser(
                latestResolution.failed.map((agent) => agent.name),
                latestResolution.unresolved.map((agent) => agent.name),
              ),
              opener: latestOpener,
            });
          })()
            .catch((error) => {
              console.warn("Failed to finish the Welcome team kickoff.", error);
            })
            .finally(() => {
              if (closerAbortControllers.get(channelId) === controller) {
                closerAbortControllers.delete(channelId);
              }
              closerInFlight.delete(channelId);
            });
        }, waitMs);
        closerTimeouts.set(channelId, timeout);
      }
      return;
    }

    const timeout = closerTimeouts.get(channelId);
    if (timeout) globalThis.clearTimeout(timeout);
    closerTimeouts.delete(channelId);

    const controller = new AbortController();
    closerAbortControllers.set(channelId, controller);
    closerInFlight.add(channelId);
    void (async () => {
      if (
        !(await waitForWelcomeKickoffBeat({
          signal: controller.signal,
          waitMs: CLOSER_BEAT_MS,
        })) ||
        controller.signal.aborted ||
        focusedWelcomeChannelRef.current !== channelId
      )
        return;

      const latestEvents = channelEventsRef.current;
      const latestOpener = markerEvent(latestEvents, openerMarker) ?? opener;
      const latestAgentSet = await resolveLatestWelcomeAgentSet({
        fallback: agentSet,
        queryClient,
        relayUrl: activeCommunity?.relayUrl,
      });
      const latestResolution = classifyWelcomeKickoffResolution(
        latestEvents,
        latestOpener,
        latestAgentSet,
      );
      await sendWelcomeKickoffCloser({
        agentSet: latestAgentSet,
        channelId,
        content: buildWelcomeKickoffCloser(
          latestResolution.failed.map((agent) => agent.name),
        ),
        opener: latestOpener,
      });
    })()
      .catch((error) => {
        console.warn("Failed to finish the Welcome team kickoff.", error);
      })
      .finally(() => {
        if (closerAbortControllers.get(channelId) === controller) {
          closerAbortControllers.delete(channelId);
        }
        closerInFlight.delete(channelId);
      });
  }, [
    activeCommunity?.relayUrl,
    agentSet,
    kickoffEvents,
    kickoffResolved,
    channelId,
    isActiveWelcome,
    queryClient,
  ]);
}

export type { WelcomeTeamStarterDefinition };
