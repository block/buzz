import { IconHash, IconMoon, IconPlus, IconSun } from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAgentsWorkspace } from "@/features/agents/ui/AgentsWorkspace";
import { runtime } from "@/shared/runtime/client";
import { DockWorkspace } from "@/shared/ui/DockWorkspace";
import {
  listSessions,
  rememberSession,
  updateSession,
} from "../sessionRegistry";
import type { Channel, Identity, Message, SessionRecord } from "../types";
import { NewSessionView } from "./NewSessionView";
import { SessionView, useAgentTurns } from "./SessionView";

type View =
  | { type: "empty" }
  | { type: "channel"; channelId: string }
  | { type: "new"; originChannelId: string }
  | { type: "session"; channelId: string };

type Destination = "channels" | "agents";

function ChannelNavigator({
  channels,
  sessions,
  selectedId,
  onOpen,
  onOpenSession,
  onNewSession,
  query,
  onQueryChange,
}: {
  channels: Channel[];
  sessions: SessionRecord[];
  selectedId?: string;
  onOpen: (channelId: string) => void;
  onOpenSession: (channelId: string) => void;
  onNewSession: (channelId: string) => void;
  query: string;
  onQueryChange: (query: string) => void;
}) {
  const sessionIds = new Set(sessions.map((session) => session.channelId));
  return (
    <aside className="workspace-navigator" aria-label="Channels">
      <header className="panel-heading navigator-heading">
        <label className="navigator-search">
          <span className="sr-only">Find a channel or Session</span>
          <input
            placeholder="Find a channel or Session"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
          />
        </label>
      </header>
      <div className="navigator-list">
        <p className="navigator-section-label text-body-sm text-tertiary">
          Channels and Sessions
        </p>
        {channels
          .filter((channel) => !sessionIds.has(channel.id))
          .filter((channel) => {
            const normalized = query.trim().toLowerCase();
            if (!normalized) return true;
            return (
              channel.name.toLowerCase().includes(normalized) ||
              sessions
                .filter((session) => session.originChannelId === channel.id)
                .some((session) =>
                  channels
                    .find((candidate) => candidate.id === session.channelId)
                    ?.name.toLowerCase()
                    .includes(normalized),
                )
            );
          })
          .map((channel) => {
            const channelSessions = sessions.filter(
              (session) => session.originChannelId === channel.id,
            );
            return (
              <section className="channel-cluster" key={channel.id}>
                <div className="channel-main-row">
                  <button
                    type="button"
                    className="navigator-row"
                    data-selected={selectedId === channel.id || undefined}
                    onClick={() => onOpen(channel.id)}
                  >
                    <IconHash size={15} stroke={1.6} aria-hidden="true" />
                    <span>{channel.name}</span>
                  </button>
                  <button
                    type="button"
                    className="navigator-row-action"
                    aria-label={`New Session in ${channel.name}`}
                    onClick={() => onNewSession(channel.id)}
                  >
                    <IconPlus size={14} stroke={1.7} aria-hidden="true" />
                  </button>
                </div>
                {channelSessions.length ? (
                  <div className="session-children">
                    {channelSessions.map((session) => {
                      const sessionChannel = channels.find(
                        (candidate) => candidate.id === session.channelId,
                      );
                      if (!sessionChannel) return null;
                      return (
                        <button
                          key={session.channelId}
                          type="button"
                          className="session-child-row"
                          data-selected={
                            selectedId === session.channelId || undefined
                          }
                          onClick={() => onOpenSession(session.channelId)}
                        >
                          <span
                            aria-hidden="true"
                            className="session-thread-line"
                          />
                          <span>{sessionChannel.name}</span>
                        </button>
                      );
                    })}
                  </div>
                ) : null}
              </section>
            );
          })}
      </div>
    </aside>
  );
}

export function AppShell() {
  const [identity, setIdentity] = useState<Identity | null>(null);
  const [relay, setRelay] = useState("");
  const [channels, setChannels] = useState<Channel[]>([]);
  const [sessions, setSessions] = useState<SessionRecord[]>([]);
  const [view, setView] = useState<View>({ type: "empty" });
  const [destination, setDestination] = useState<Destination>("channels");
  const [pending, setPending] = useState<Message>();
  const [bootstrapError, setBootstrapError] = useState<string | null>(null);
  const [dark, setDark] = useState(false);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [channelQuery, setChannelQuery] = useState("");
  const turns = useAgentTurns();
  const agentsWorkspace = useAgentsWorkspace();
  const scope = identity ? `${relay}:${identity.pubkey}` : "";

  const refresh = useCallback(async () => {
    const [nextIdentity, nextRelay, nextChannels, pendingBootstraps] =
      await Promise.all([
        runtime.identity(),
        runtime.relayUrl(),
        runtime.channels(),
        runtime.pendingSessionBootstraps(),
      ]);
    let effectiveChannels = nextChannels;
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
    const nextScope = `${nextRelay}:${nextIdentity.pubkey}`;
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
    setIdentity(nextIdentity);
    setRelay(nextRelay);
    setChannels([
      ...incompleteChannels,
      ...discoveredChannels,
      ...effectiveChannels.filter(
        (channel) =>
          !incompleteChannels.some(
            (incomplete) => incomplete.id === channel.id,
          ) && !discoveredChannels.some((session) => session.id === channel.id),
      ),
    ]);
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
    setSessions(listSessions(nextScope));
    setStartupError(null);
  }, []);

  useEffect(() => {
    void refresh().catch((caught) => {
      setStartupError(
        caught instanceof Error ? caught.message : String(caught),
      );
    });
  }, [refresh]);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
  }, [dark]);

  const channelsById = useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  async function createSession(content: string) {
    if (view.type !== "new") return;
    const originChannelId = view.originChannelId;
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
      setView({ type: "session", channelId: created.id });
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
    setView({ type: "session", channelId });
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
          setView({ type: "channel", channelId: view.originChannelId })
        }
        onCreate={createSession}
      />
    ) : activeChannel &&
      (view.type === "channel" || view.type === "session") ? (
      <SessionView
        channel={activeChannel}
        identity={identity}
        origin={origin}
        turns={turns}
        mode={view.type}
        onStartSession={
          view.type === "channel"
            ? () => setView({ type: "new", originChannelId: activeChannel.id })
            : undefined
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
            onClick={() => setDestination("channels")}
          >
            Channels
          </button>
          <button
            type="button"
            data-selected={destination === "agents" || undefined}
            onClick={() => setDestination("agents")}
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
            onClick={() => setDark((value) => !value)}
            aria-label={dark ? "Use light mode" : "Use dark mode"}
          >
            {dark ? (
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
                <ChannelNavigator
                  channels={channels}
                  sessions={sessions}
                  selectedId={activeId}
                  onOpen={(channelId) => {
                    setDestination("channels");
                    setView({ type: "channel", channelId });
                  }}
                  onOpenSession={openSession}
                  onNewSession={(originChannelId) =>
                    setView({ type: "new", originChannelId })
                  }
                  query={channelQuery}
                  onQueryChange={setChannelQuery}
                />
              ) : (
                agentsWorkspace.navigator
              ),
            conversation,
          }}
        />
      </section>
    </div>
  );
}
