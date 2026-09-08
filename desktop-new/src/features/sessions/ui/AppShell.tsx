import { IconHash, IconMoon, IconSun } from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAgentActivity } from "@/features/agent-activity/useAgentActivity";
import { useComposerDrafts } from "@/features/composer/useComposerDrafts";
import { useAgentsWorkspace } from "@/features/agents/ui/AgentsWorkspace";
import { MessagesNavigator } from "@/features/navigation/ui/MessagesNavigator";
import { useNavigation } from "@/features/navigation/useNavigation";
import { communityScope, useCommunity } from "@/shared/community/useCommunity";
import { useIdentity } from "@/shared/identity/useIdentity";
import { isMockRuntime, runtime } from "@/shared/runtime/client";
import { useColorScheme } from "@/shared/theme/useColorScheme";
import { DockWorkspace } from "@/shared/ui/DockWorkspace";
import { Panel } from "@/shared/ui/Panel";
import {
  listSessions,
  rememberSession,
  updateSession,
} from "../sessionRegistry";
import type { Channel, Message, SessionRecord } from "../types";
import { NewSessionView } from "./NewSessionView";
import { SessionView } from "./SessionView";

export function AppShell() {
  const { identity, load: loadIdentity } = useIdentity();
  const { relayUrl: relay, load: loadCommunity } = useCommunity();
  const { scheme, toggle: toggleColorScheme } = useColorScheme();
  const [channels, setChannels] = useState<Channel[]>([]);
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [pending, setPending] = useState<Message>();
  const [bootstrapError, setBootstrapError] = useState<string | null>(null);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [channelQuery, setChannelQuery] = useState("");
  const {
    destination,
    messagesDestination: view,
    resolveMessages,
    openAgents,
    openChannel,
    openChannels,
    openSession: navigateToSession,
    startSession,
  } = useNavigation();
  const agentActivity = useAgentActivity();
  const composerDrafts = useComposerDrafts();
  const agentsWorkspace = useAgentsWorkspace();
  const scope = identity ? communityScope(relay, identity.pubkey) : "";

  const refresh = useCallback(async () => {
    const [nextIdentity, nextRelay, loadedChannels, pendingBootstraps] =
      await Promise.all([
        loadIdentity(),
        loadCommunity(),
        runtime.channels(),
        runtime.pendingSessionBootstraps(),
      ]);
    let effectiveChannels = loadedChannels;
    const failedRecoveries = (
      await Promise.all(
        pendingBootstraps.map(async (operation) => {
          try {
            await runtime.resumeSessionBootstrap(operation);
            return null;
          } catch (error) {
            return { operation, error };
          }
        }),
      )
    ).filter((failure) => failure !== null);
    if (pendingBootstraps.length > failedRecoveries.length) {
      effectiveChannels = await runtime.channels();
    }
    const nextScope = communityScope(nextRelay, nextIdentity.pubkey);
    const discoveredLinks = (
      await Promise.all(
        effectiveChannels.map((parent) =>
          runtime
            .channelSessions(parent.id)
            .then((links) =>
              links.map((link) => ({ ...link, parentChannelId: parent.id })),
            ),
        ),
      )
    ).flat();
    const discoveredChannels: Channel[] = discoveredLinks.flatMap((link) =>
      link.channel
        ? [
            {
              id: link.channel.id,
              name: link.channel.name,
              channelType: link.channel.channel_type,
              visibility: link.channel.visibility,
              description: link.channel.description,
              memberCount: link.channel.member_count,
              lastMessageAt: null,
            },
          ]
        : [],
    );
    const incompleteChannels: Channel[] = failedRecoveries.map(
      ({ operation }) => ({
        id: operation.channel_id,
        name: "Session needs attention",
        channelType: "stream",
        visibility: "private",
        description: "",
        memberCount: 1,
        lastMessageAt: null,
      }),
    );
    const nextChannels = [
      ...incompleteChannels,
      ...discoveredChannels,
      ...effectiveChannels.filter(
        (channel) =>
          !incompleteChannels.some(
            (incomplete) => incomplete.id === channel.id,
          ) && !discoveredChannels.some((session) => session.id === channel.id),
      ),
    ];
    setChannels(nextChannels);
    resolveMessages(
      nextChannels,
      new Set([
        ...incompleteChannels.map((channel) => channel.id),
        ...discoveredChannels.map((channel) => channel.id),
      ]),
    );
    for (const link of discoveredLinks) {
      rememberSession(nextScope, {
        channelId: link.session_channel_id,
        originChannelId: link.parentChannelId,
        createdAt: link.created_at * 1000,
        updatedAt: link.created_at * 1000,
      });
    }
    for (const { operation } of failedRecoveries) {
      if (
        operation.relay_url.includes(new URL(nextRelay).host) &&
        operation.signer_pubkey === nextIdentity.pubkey
      ) {
        rememberSession(nextScope, {
          channelId: operation.channel_id,
          originChannelId: operation.parent_channel_id,
          createdAt: Date.now(),
          updatedAt: Date.now(),
          incompleteDraft: operation.content,
        });
      }
    }
    const rememberedSessions = listSessions(nextScope);
    // Rich browser fixtures give the navigation a real standalone Session to
    // arrange and open without claiming the current native parent-required
    // bootstrap contract can create one yet.
    const fixtureSessions = isMockRuntime
      ? [
          {
            channelId: "session-agent-setup",
            createdAt: Date.now() - 10_800_000,
            updatedAt: Date.now() - 10_800_000,
          },
        ]
      : [];
    setSessions([
      ...fixtureSessions,
      ...rememberedSessions.filter(
        (remembered) =>
          !fixtureSessions.some(
            (fixture) => fixture.channelId === remembered.channelId,
          ),
      ),
    ]);
    setStartupError(null);
  }, [loadCommunity, loadIdentity, resolveMessages]);

  useEffect(() => {
    void refresh().catch((caught) => {
      setStartupError(
        caught instanceof Error ? caught.message : String(caught),
      );
    });
  }, [refresh]);

  const channelsById = useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  async function createSession(content: string) {
    if (view.type !== "new") return;
    const originChannelId = view.originChannelId;
    if (!originChannelId) {
      const message =
        "Standalone Sessions need the native creation contract before they can be started here.";
      setBootstrapError(message);
      throw new Error(message);
    }
    const operationId =
      pending?.pending === "failed"
        ? pending.id
        : `bootstrap-${crypto.randomUUID()}`;
    const optimistic: Message = {
      id: operationId,
      pubkey: identity?.pubkey ?? "",
      content,
      createdAt: Math.floor(Date.now() / 1000),
      kind: 9,
      tags: [],
      pending: "creating",
    };
    setPending(optimistic);
    setBootstrapError(null);
    try {
      const created = await runtime.createSession(
        content,
        originChannelId,
        operationId,
      );
      rememberSession(scope, {
        channelId: created.id,
        originChannelId,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      });
      setChannels((current) => [created, ...current]);
      setSessions(listSessions(scope));
      setPending(undefined);
      navigateToSession(created.id);
    } catch (caught) {
      const message = caught instanceof Error ? caught.message : String(caught);
      setPending({ ...optimistic, pending: "failed", error: message });
      setBootstrapError(message);
      throw caught;
    }
  }

  function openSession(channelId: string) {
    updateSession(scope, channelId, { updatedAt: Date.now() });
    setSessions(listSessions(scope));
    navigateToSession(channelId);
  }

  if (!identity) {
    return (
      <div className="app-loading text-body text-secondary">
        {startupError ? (
          <div role="alert">
            <p>Buzz could not connect.</p>
            <button type="button" onClick={() => void refresh()}>
              Try again
            </button>
          </div>
        ) : (
          "Connecting to Buzz…"
        )}
      </div>
    );
  }

  const draftDestinationId =
    view.type === "channel" || view.type === "session"
      ? `conversation:${view.channelId}`
      : view.type === "new"
        ? `new-session:${view.originChannelId ?? "standalone"}`
        : undefined;
  const draft = draftDestinationId
    ? composerDrafts.readDraft(draftDestinationId)
    : "";

  const activeId =
    view.type === "channel" || view.type === "session"
      ? view.channelId
      : undefined;
  const activeChannel = activeId ? channelsById.get(activeId) : undefined;
  const activeSession =
    view.type === "session"
      ? sessions.find((session) => session.channelId === view.channelId)
      : undefined;
  const originId =
    view.type === "new" ? view.originChannelId : activeSession?.originChannelId;
  const origin = originId ? channelsById.get(originId) : undefined;

  const conversation =
    destination === "agents" ? (
      agentsWorkspace.content
    ) : view.type === "new" ? (
      <NewSessionView
        origin={origin}
        pending={pending}
        error={bootstrapError}
        onBack={() =>
          view.originChannelId
            ? openChannel(view.originChannelId)
            : openChannels()
        }
        onCreate={createSession}
        draft={draft}
        onDraftChange={(value) =>
          composerDrafts.writeDraft(
            `new-session:${view.originChannelId ?? "standalone"}`,
            value,
          )
        }
      />
    ) : activeChannel &&
      (view.type === "channel" || view.type === "session") ? (
      <SessionView
        channel={activeChannel}
        identity={identity}
        origin={origin}
        turns={agentActivity.forChannel(activeChannel.id)}
        mode={view.type}
        onStartSession={
          view.type === "channel"
            ? () => startSession(activeChannel.id)
            : undefined
        }
        draft={draft}
        onDraftChange={(value) =>
          composerDrafts.writeDraft(`conversation:${activeChannel.id}`, value)
        }
      />
    ) : (
      <main className="empty-workspace-panel">
        <IconHash size={22} stroke={1.4} aria-hidden="true" />
        <h1 className="text-heading text-primary">Choose a channel</h1>
        <p className="text-body text-secondary">
          Open a conversation or begin a focused Session inside it.
        </p>
      </main>
    );

  return (
    <div className="workspace-shell">
      <header className="workspace-topbar">
        <nav aria-label="Workspace destinations" className="destination-tabs">
          <button
            type="button"
            data-selected={destination === "channels" || undefined}
            onClick={openChannels}
          >
            Channels
          </button>
          <button
            type="button"
            data-selected={destination === "agents" || undefined}
            onClick={openAgents}
          >
            Agents
          </button>
        </nav>
        <div className="workspace-identity">
          <span className="text-body-sm text-secondary">
            {identity.displayName}
          </span>
          <button
            type="button"
            className="topbar-icon-button"
            onClick={toggleColorScheme}
            aria-label={scheme === "dark" ? "Use light mode" : "Use dark mode"}
          >
            {scheme === "dark" ? (
              <IconSun size={16} aria-hidden="true" />
            ) : (
              <IconMoon size={16} aria-hidden="true" />
            )}
          </button>
        </div>
      </header>
      <section className="workspace-stage">
        <DockWorkspace
          panels={{
            navigator:
              destination === "channels" ? (
                <Panel as="aside" aria-label="Channels">
                  <MessagesNavigator
                    conversations={channels}
                    sessions={sessions}
                    selectedId={activeId}
                    onOpenRoom={openChannel}
                    onOpenDirectMessage={openChannel}
                    onOpenSession={openSession}
                    onStartSession={startSession}
                    onStartStandaloneSession={() => startSession()}
                    query={channelQuery}
                    onQueryChange={setChannelQuery}
                  />
                </Panel>
              ) : (
                <Panel as="aside" aria-label="Agents">
                  {agentsWorkspace.navigator}
                </Panel>
              ),
            conversation: (
              <Panel aria-label="Workspace content">{conversation}</Panel>
            ),
          }}
        />
      </section>
    </div>
  );
}
