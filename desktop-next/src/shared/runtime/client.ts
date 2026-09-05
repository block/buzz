import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { projectChannelWindow } from "@/features/sessions/channelWindowProjection";
import type {
  Channel as BuzzChannel,
  Identity,
  Message,
  ObserverEvent,
  Participant,
} from "@/features/sessions/types";

type RawIdentity = { pubkey: string; display_name: string };
type RawChannel = {
  id: string;
  name: string;
  channel_type: string;
  visibility: string;
  description: string;
  member_count: number;
  last_message_at?: string | null;
};
type RawChannels = { hash: string; channels: RawChannel[] | null };
type RawMember = {
  pubkey: string;
  role: string;
  is_agent: boolean;
  display_name?: string | null;
};
type RawEvent = {
  id: string;
  pubkey: string;
  content: string;
  created_at: number;
  kind: number;
  tags: string[][];
};

const LIVE_MESSAGE_KINDS = [9, 40002, 40008];

type MockState = {
  identity: Identity;
  channels: BuzzChannel[];
  messages: Record<string, Message[]>;
  participants: Record<string, Participant[]>;
};

const mockState: MockState = {
  identity: { pubkey: "morgan-pubkey", displayName: "Morgan" },
  channels: [
    {
      id: "design",
      name: "design",
      channelType: "stream",
      visibility: "private",
      description: "Interface direction and product design",
      memberCount: 4,
      lastMessageAt: new Date().toISOString(),
    },
    {
      id: "buzz-interface",
      name: "buzz-interface",
      channelType: "stream",
      visibility: "private",
      description: "The team building Buzz together",
      memberCount: 7,
      lastMessageAt: new Date(Date.now() - 86_400_000).toISOString(),
    },
  ],
  messages: {
    design: [
      {
        id: "message-1",
        pubkey: "cynthia-pubkey",
        content: "The session should begin as simply as a thought: just type.",
        createdAt: Math.floor(Date.now() / 1000) - 360,
        kind: 9,
        tags: [["h", "design"]],
      },
    ],
  },
  participants: {
    design: [
      {
        pubkey: "morgan-pubkey",
        role: "owner",
        isAgent: false,
        displayName: "Morgan",
      },
      {
        pubkey: "cynthia-pubkey",
        role: "member",
        isAgent: false,
        displayName: "Cynthia",
      },
      {
        pubkey: "vogue-agent",
        role: "bot",
        isAgent: true,
        displayName: "Vogue",
      },
    ],
  },
};

const useMock = !isTauri() || new URLSearchParams(location.search).has("mock");

function relayFrames(delivery: unknown): unknown[] {
  const envelopes = Array.isArray(delivery) ? delivery : [delivery];
  return envelopes.flatMap((envelope) => {
    if (
      typeof envelope === "object" &&
      envelope !== null &&
      "type" in envelope &&
      envelope.type === "Text" &&
      "data" in envelope &&
      typeof envelope.data === "string"
    ) {
      try {
        return [JSON.parse(envelope.data)];
      } catch {
        return [];
      }
    }
    return [];
  });
}

async function subscribeRelay(
  filter: Record<string, unknown>,
  onEvent: (event: RawEvent) => void,
): Promise<() => void> {
  const relayUrl = await invoke<string>("get_relay_ws_url");
  const subscriptionId = `desktop-next-${crypto.randomUUID()}`;
  let connectionId: number | null = null;
  let subscribed = false;
  const send = async (frame: unknown[]) => {
    if (connectionId === null) return;
    await invoke("plugin:websocket|send", {
      id: connectionId,
      message: { type: "Text", data: JSON.stringify(frame) },
    });
  };
  const inbound = new Channel<unknown>((delivery) => {
    for (const frame of relayFrames(delivery)) {
      if (!Array.isArray(frame)) continue;
      if (frame[0] === "AUTH" && typeof frame[1] === "string") {
        void invoke<string>("create_auth_event", {
          challenge: frame[1],
          relayUrl,
        }).then((eventJson) => send(["AUTH", JSON.parse(eventJson)]));
        continue;
      }
      if (frame[0] === "OK" && frame[2] === true && !subscribed) {
        subscribed = true;
        void send(["REQ", subscriptionId, filter]);
        continue;
      }
      if (frame[0] === "EVENT" && frame[1] === subscriptionId && frame[2]) {
        onEvent(frame[2] as RawEvent);
      }
    }
  });
  connectionId = await invoke<number>("plugin:websocket|connect", {
    url: relayUrl,
    onMessage: inbound,
    config: {},
  });
  return () => {
    if (connectionId === null) return;
    void send(["CLOSE", subscriptionId]).finally(() =>
      invoke("plugin:websocket|disconnect", { id: connectionId }),
    );
  };
}

function channel(raw: RawChannel): BuzzChannel {
  return {
    id: raw.id,
    name: raw.name,
    channelType: raw.channel_type,
    visibility: raw.visibility,
    description: raw.description,
    memberCount: raw.member_count,
    lastMessageAt: raw.last_message_at,
  };
}

function message(raw: RawEvent): Message {
  return {
    id: raw.id,
    pubkey: raw.pubkey,
    content: raw.content,
    createdAt: raw.created_at,
    kind: raw.kind,
    tags: raw.tags,
  };
}

export const runtime = {
  async identity(): Promise<Identity> {
    if (useMock) return mockState.identity;
    const raw = await invoke<RawIdentity>("get_identity");
    return { pubkey: raw.pubkey, displayName: raw.display_name };
  },

  async relayUrl(): Promise<string> {
    if (useMock) return "mock://buzz-community";
    return invoke<string>("get_relay_ws_url");
  },

  async pendingSessionBootstraps() {
    if (useMock) return [];
    return invoke<
      {
        operation_id: string;
        relay_url: string;
        signer_pubkey: string;
        channel_id: string;
        parent_channel_id: string;
        content: string;
      }[]
    >("list_pending_session_bootstraps");
  },

  async channelSessions(parentChannelId: string) {
    if (useMock) return [];
    return invoke<
      {
        parent_channel_id: string;
        session_channel_id: string;
        creator_pubkey: string;
        created_at: number;
        channel: {
          id: string;
          name: string;
          channel_type: string;
          visibility: string;
          description: string;
          member_count: number;
        } | null;
      }[]
    >("list_channel_sessions", { parentChannelId });
  },

  async resumeSessionBootstrap(operation: {
    operation_id: string;
    content: string;
    parent_channel_id: string;
  }): Promise<void> {
    if (useMock) return;
    const [relay, identity] = await Promise.all([
      this.relayUrl(),
      this.identity(),
    ]);
    await invoke("bootstrap_session", {
      operationId: operation.operation_id,
      content: operation.content,
      parentChannelId: operation.parent_channel_id,
      expectedRelayUrl: relay,
      expectedSignerPubkey: identity.pubkey,
    });
  },

  async channels(): Promise<BuzzChannel[]> {
    if (useMock) return [...mockState.channels];
    const raw = await invoke<RawChannels>("get_channels", { knownHash: null });
    return (raw.channels ?? []).map(channel);
  },

  async messages(channelId: string): Promise<Message[]> {
    if (useMock) return [...(mockState.messages[channelId] ?? [])];
    const events = await invoke<RawEvent[]>("get_channel_window", {
      channelId,
      limitRows: 80,
      cursor: null,
    });
    return projectChannelWindow(events).messages;
  },

  async participants(channelId: string): Promise<Participant[]> {
    if (useMock) return [...(mockState.participants[channelId] ?? [])];
    const response = await invoke<{ members: RawMember[] }>(
      "get_channel_members",
      {
        channelId,
      },
    );
    return response.members.map((member) => ({
      pubkey: member.pubkey,
      role: member.role,
      isAgent: member.is_agent,
      displayName: member.display_name,
    }));
  },

  async createSession(
    firstMessage: string,
    parentChannelId: string,
    operationId: string = crypto.randomUUID(),
  ): Promise<BuzzChannel> {
    if (useMock) {
      const id = crypto.randomUUID();
      const created: BuzzChannel = {
        id,
        name: firstMessage.trim().slice(0, 56) || "New session",
        channelType: "stream",
        visibility: "private",
        description: "",
        memberCount: 1,
        lastMessageAt: new Date().toISOString(),
      };
      mockState.channels.unshift(created);
      mockState.messages[id] = [];
      mockState.participants[id] = [
        { ...mockState.identity, role: "owner", isAgent: false },
      ];
      await this.sendMessage(id, firstMessage);
      return created;
    }
    const identity = await this.identity();
    const relay = await invoke<string>("get_relay_http_url");
    const created = await invoke<{
      channel_id: string;
      created_at: number;
    }>("bootstrap_session", {
      operationId,
      content: firstMessage,
      parentChannelId,
      expectedRelayUrl: relay,
      expectedSignerPubkey: identity.pubkey,
    });
    return {
      id: created.channel_id,
      name: firstMessage.trim().slice(0, 56) || "New session",
      channelType: "stream",
      visibility: "private",
      description: "",
      memberCount: 1,
      lastMessageAt: new Date(created.created_at * 1000).toISOString(),
    };
  },

  async sendMessage(channelId: string, content: string): Promise<Message> {
    if (useMock) {
      const created: Message = {
        id: crypto.randomUUID(),
        pubkey: mockState.identity.pubkey,
        content,
        createdAt: Math.floor(Date.now() / 1000),
        kind: 9,
        tags: [["h", channelId]],
      };
      mockState.messages[channelId] = [
        ...(mockState.messages[channelId] ?? []),
        created,
      ];
      return created;
    }
    const identity = await this.identity();
    const relay = await invoke<string>("get_relay_http_url");
    const response = await invoke<{ event_id: string; created_at: number }>(
      "send_channel_message",
      {
        channelId,
        content,
        parentEventId: null,
        rootEventId: null,
        mediaTags: null,
        emojiTags: null,
        mentionTags: null,
        linkPreviewTags: null,
        sentFromThreadTag: null,
        mentionPubkeys: null,
        kind: 9,
        expectedRelayUrl: relay,
        expectedSignerPubkey: identity.pubkey,
      },
    );
    return {
      id: response.event_id,
      pubkey: identity.pubkey,
      content,
      createdAt: response.created_at,
      kind: 9,
      tags: [["h", channelId]],
    };
  },

  async subscribeMessages(
    channelId: string,
    onMessage: (message: Message) => void,
  ) {
    if (useMock) return () => undefined;
    return subscribeRelay(
      {
        kinds: LIVE_MESSAGE_KINDS,
        "#h": [channelId],
        since: Math.floor(Date.now() / 1000),
      },
      (event) => onMessage(message(event)),
    );
  },

  async searchPeople(query: string) {
    if (useMock) {
      return [
        { pubkey: "vogue-agent", display_name: "Vogue", is_agent: true },
        { pubkey: "cynthia-pubkey", display_name: "Cynthia", is_agent: false },
      ].filter((person) =>
        person.display_name.toLowerCase().includes(query.toLowerCase()),
      );
    }
    const [result, managedAgents] = await Promise.all([
      invoke<{ users: Record<string, unknown>[] }>("search_users", {
        query,
        limit: 8,
        cursor: null,
      }),
      invoke<{ pubkey: string; name: string }[]>("list_managed_agents"),
    ]);
    const normalized = query.trim().toLowerCase();
    const agents = managedAgents
      .filter((agent) => agent.name.toLowerCase().includes(normalized))
      .map((agent) => ({
        pubkey: agent.pubkey,
        display_name: agent.name,
        is_agent: true,
      }));
    const people = result.users as {
      pubkey: string;
      display_name?: string;
      is_agent?: boolean;
    }[];
    return [
      ...new Map(
        [...agents, ...people].map((person) => [person.pubkey, person]),
      ).values(),
    ];
  },

  async addParticipant(channelId: string, pubkey: string, isAgent: boolean) {
    if (useMock) return { added: [pubkey], errors: [] };
    const identity = await this.identity();
    const managedAgents = isAgent
      ? await invoke<{ pubkey: string; status: string }[]>(
          "list_managed_agents",
        )
      : [];
    const managedAgent = managedAgents.find(
      (candidate) => candidate.pubkey === pubkey,
    );
    const replayFloorUnix = Math.floor(Date.now() / 1000);
    const relay = await invoke<string>("get_relay_http_url");
    const outcome = await invoke<{ added: string[]; errors: unknown[] }>(
      "add_channel_members",
      {
        channelId,
        pubkeys: [pubkey],
        role: isAgent ? "bot" : "member",
        expectedRelayUrl: relay,
        expectedSignerPubkey: identity.pubkey,
      },
    );
    if (
      managedAgent &&
      managedAgent.status !== "running" &&
      managedAgent.status !== "deployed" &&
      outcome.errors.length === 0
    ) {
      await invoke("start_managed_agent", {
        pubkey,
        expectedRelayUrl: await invoke<string>("get_relay_ws_url"),
        expectedSignerPubkey: identity.pubkey,
        replayFloorUnix,
      });
    }
    return outcome;
  },

  async observe(onEvent: (event: ObserverEvent) => void) {
    if (useMock) {
      const events: ObserverEvent[] = [
        {
          seq: 1,
          timestamp: new Date().toISOString(),
          kind: "turn_started",
          channelId: "design",
          sessionId: "vogue-session",
          turnId: "turn-1",
          agentPubkey: "vogue-agent",
          payload: {
            agentName: "Vogue",
            title: "Reviewing the Session flow",
            status: "running",
          },
        },
        ...[
          "Read the interaction plan",
          "Checked participant states",
          "Refined the conversation hierarchy",
          "Validated narrow layout",
        ].map((title, index) => ({
          seq: index + 2,
          timestamp: new Date(Date.now() + index).toISOString(),
          kind: "tool_call_completed",
          channelId: "design",
          sessionId: "vogue-session",
          turnId: "turn-1",
          agentPubkey: "vogue-agent",
          payload: {
            agentName: "Vogue",
            title,
            toolCallId: `tool-${index}`,
            status: "completed",
          },
        })),
      ];
      const timer = window.setTimeout(() => events.forEach(onEvent), 120);
      return () => window.clearTimeout(timer);
    }
    const identity = await this.identity();
    return subscribeRelay(
      {
        kinds: [24200],
        "#p": [identity.pubkey],
        since: Math.floor(Date.now() / 1000),
      },
      async (signedEvent) => {
        try {
          const decrypted = await invoke<ObserverEvent>(
            "decrypt_observer_event",
            {
              eventJson: JSON.stringify(signedEvent),
            },
          );
          const nested =
            decrypted.kind === "batch" &&
            Array.isArray(decrypted.payload.events)
              ? (decrypted.payload.events as ObserverEvent[])
              : [decrypted];
          for (const event of nested) {
            onEvent({ ...event, agentPubkey: signedEvent.pubkey });
          }
        } catch {
          // Invalid, forged, or non-owner observer events are ignored.
        }
      },
    );
  },
};
