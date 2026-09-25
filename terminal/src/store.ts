import { randomUUID } from "node:crypto";
import type {
  HostMessage,
  MessageRequest,
  ObserverEnvelope,
  RelayEvent,
} from "./protocol.ts";
import { HostError, type Transport } from "./transport.ts";

export const MAX_CHANNELS = 100;
export const MAX_VIEWS = 24;
export const HISTORY_LIMIT = 200;

/** A relay-authoritative channel roster and name. */
export interface Channel {
  id: string;
  name: string;
  members: string[];
  joined: boolean;
  roster?: RelayEvent;
  metadata?: RelayEvent;
}

/** Human or agent identity, with ownership verified by the signing host. */
export interface Profile {
  pubkey: string;
  name: string;
  owner?: string;
  event: RelayEvent;
}

/** Local conversation state. This is not an execution session. */
export interface Conversation {
  key: string;
  channelId: string;
  rootEventId?: string;
  draft: string;
  recipient?: string;
  unread: number;
  history: "loading" | "ready" | "error";
  error?: string;
  sending: boolean;
  pending?: MessageRequest;
}

/** Best-effort execution status keyed by agent, runtime session, and turn. */
export interface Activity {
  key: string;
  agent: string;
  channelId: string;
  sessionId: string;
  turnId: string;
  state: "working" | "finished" | "failed" | "unknown";
  detail: string;
  updatedAt: number;
  seq: number;
}

/** Read one Nostr tag without interpreting names as identities. */
export function tag(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((item) => item[0] === name)?.[1];
}

/** Buzz direct replies mark the root as reply; nested replies carry a root tag. */
export function threadRoot(event: RelayEvent): string | undefined {
  return (
    event.tags.find((item) => item[0] === "e" && item[3] === "root")?.[1] ??
    event.tags.find((item) => item[0] === "e" && item[3] === "reply")?.[1]
  );
}

function newer(event: RelayEvent, prior?: RelayEvent): boolean {
  return (
    !prior ||
    event.created_at > prior.created_at ||
    (event.created_at === prior.created_at && event.id < prior.id)
  );
}

/** Community-scoped state and delivery operations, independent of presentation. */
export class Store {
  pubkey = "";
  relayPubkey = "";
  relayUrl = "";
  connection: "connecting" | "connected" | "disconnected" | "failed" =
    "connecting";
  notice = "Connecting to your community…";
  channels = new Map<string, Channel>();
  profiles = new Map<string, Profile>();
  views = new Map<string, Conversation>();
  messages = new Map<string, RelayEvent[]>();
  activity = new Map<string, Activity>();
  activeKey?: string;
  private listeners = new Set<() => void>();
  private live = new Set<string>();
  private observerIds = new Set<string>();

  /** Register a state listener; the returned disposer removes it. */
  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Notify subscribers after a local edit or selection. */
  changed(): void {
    for (const listener of this.listeners) listener();
  }

  get current(): Conversation | undefined {
    return this.activeKey ? this.views.get(this.activeKey) : undefined;
  }

  /** Resolve a display label while preserving a pubkey fallback. */
  name(pubkey: string): string {
    return pubkey === this.pubkey
      ? "You"
      : this.profiles.get(pubkey)?.name || pubkey.slice(0, 12);
  }

  /** Open a local view without altering any agent execution or another draft. */
  open(channelId: string, rootEventId?: string): Conversation {
    if (!this.channels.get(channelId)?.joined)
      throw new Error("You are no longer a member of this channel.");
    const key = `${channelId}:${rootEventId ?? ""}`;
    let view = this.views.get(key);
    if (!view) {
      if (this.views.size >= MAX_VIEWS)
        throw new Error(
          `Only ${MAX_VIEWS} conversations can be open. Close one with /close first.`,
        );
      view = {
        key,
        channelId,
        rootEventId,
        draft: "",
        unread: 0,
        history: "loading",
        sending: false,
      };
      this.views.set(key, view);
    }
    this.activeKey = key;
    view.unread = 0;
    this.changed();
    return view;
  }

  /** Select an exact roster member, never a potentially ambiguous display name. */
  selectRecipient(view: Conversation, pubkey: string): void {
    if (view.sending || view.pending)
      throw new Error(
        "Resolve the pending send before changing its recipient.",
      );
    if (!this.channels.get(view.channelId)?.members.includes(pubkey))
      throw new Error("This identity is not a member of the conversation.");
    view.recipient = pubkey;
    this.changed();
  }

  /** Return this conversation's bounded history in stable chronological order. */
  events(view: Conversation): RelayEvent[] {
    return this.messages.get(view.key) ?? [];
  }

  /** Reduce one authenticated host notification. */
  handle(message: HostMessage): void {
    if ("id" in message) return;
    if (message.type === "ready") {
      if (message.protocolVersion !== 1)
        throw new Error("Unsupported relay host protocol.");
      this.pubkey = message.pubkey;
      this.relayPubkey = message.relayPubkey;
      this.relayUrl = message.relayUrl;
    } else if (message.type === "connection") {
      this.connection = message.status;
      this.notice =
        message.message ??
        (message.status === "connected"
          ? "Connected · history is a recent window; activity is live only"
          : "Reconnecting · agents keep working independently");
      if (message.status !== "connected") {
        this.live.clear();
        for (const view of this.views.values()) view.history = "loading";
        for (const item of this.activity.values())
          if (item.state === "working") item.state = "unknown";
      }
    } else if (message.type === "eose") {
      this.live.add(message.subscriptionId);
      const view = this.views.get(message.subscriptionId);
      if (view) {
        view.history = "ready";
        view.error = undefined;
      }
    } else if (message.type === "closed") {
      this.live.delete(message.subscriptionId);
      const view = this.views.get(message.subscriptionId);
      if (view) {
        view.history = "error";
        view.error = message.message;
      }
      this.notice = `Subscription closed: ${message.message}. Use /reconnect to retry.`;
    } else if (message.type === "notice") this.notice = message.message;
    else if (message.type === "event")
      this.event(message.event, message.subscriptionId, message.ownerPubkey);
    else if (message.type === "observer")
      this.observe(message.eventId, message.agentPubkey, message.envelope);
    this.changed();
  }

  private event(
    event: RelayEvent,
    subscriptionId: string,
    owner?: string,
  ): void {
    if (event.kind === 39000 || event.kind === 39002) {
      if (event.pubkey !== this.relayPubkey) return;
      const id = tag(event, "d");
      if (!id) return;
      let channel = this.channels.get(id);
      if (!channel) {
        if (this.channels.size >= MAX_CHANNELS) {
          this.notice = "Channel limit reached (100).";
          return;
        }
        channel = { id, name: id.slice(0, 12), joined: false, members: [] };
        this.channels.set(id, channel);
      }
      if (event.kind === 39000 && newer(event, channel.metadata)) {
        channel.metadata = event;
        channel.name = tag(event, "name") || channel.name;
      } else if (event.kind === 39002 && newer(event, channel.roster)) {
        channel.roster = event;
        channel.members = [
          ...new Set(
            event.tags.filter((item) => item[0] === "p").map((item) => item[1]),
          ),
        ].slice(0, 1000);
        channel.joined = channel.members.includes(this.pubkey);
        for (const view of this.views.values()) {
          if (view.channelId !== id) continue;
          if (
            !channel.joined ||
            (view.recipient && !channel.members.includes(view.recipient))
          ) {
            view.recipient = undefined;
            view.error =
              "Membership changed. Review the recipient before sending.";
          }
        }
        if (!channel.joined) {
          for (const view of this.views.values())
            if (view.channelId === id) this.messages.delete(view.key);
          for (const [key, activity] of this.activity)
            if (activity.channelId === id) this.activity.delete(key);
        }
      }
      return;
    }
    if (event.kind === 0) {
      if (!newer(event, this.profiles.get(event.pubkey)?.event)) return;
      if (!this.profiles.has(event.pubkey) && this.profiles.size >= 1000)
        return;
      try {
        const data = JSON.parse(event.content) as Record<string, unknown>;
        const name =
          typeof data.display_name === "string"
            ? data.display_name
            : typeof data.name === "string"
              ? data.name
              : "";
        this.profiles.set(event.pubkey, {
          pubkey: event.pubkey,
          name: name.slice(0, 160),
          event,
          owner,
        });
        if (owner !== this.pubkey)
          for (const [key, item] of this.activity)
            if (item.agent === event.pubkey) this.activity.delete(key);
      } catch {
        this.notice = "An invalid identity profile was ignored.";
      }
      return;
    }
    if (![9, 40002, 40008].includes(event.kind)) return;
    const channelId = tag(event, "h");
    if (!channelId || !this.channels.get(channelId)?.joined) return;
    for (const view of this.views.values()) {
      if (view.channelId !== channelId) continue;
      if (
        view.rootEventId
          ? event.id !== view.rootEventId &&
            threadRoot(event) !== view.rootEventId
          : !!threadRoot(event)
      )
        continue;
      const events = this.messages.get(view.key) ?? [];
      if (events.some((item) => item.id === event.id)) continue;
      events.push(event);
      events.sort(
        (a, b) => a.created_at - b.created_at || a.id.localeCompare(b.id),
      );
      const root =
        view.rootEventId && events.find((item) => item.id === view.rootEventId);
      const recent = events
        .filter((item) => item !== root)
        .slice(-HISTORY_LIMIT);
      this.messages.set(view.key, root ? [root, ...recent] : recent);
      if (
        this.live.has(subscriptionId) &&
        event.pubkey !== this.pubkey &&
        view.key !== this.activeKey
      )
        view.unread++;
    }
  }

  private observe(
    eventId: string,
    agent: string,
    envelope: ObserverEnvelope,
    depth = 0,
  ): void {
    if (
      !envelope ||
      typeof envelope !== "object" ||
      typeof envelope.kind !== "string" ||
      depth > 1 ||
      this.profiles.get(agent)?.owner !== this.pubkey ||
      this.observerIds.has(eventId)
    )
      return;
    this.observerIds.add(eventId);
    if (this.observerIds.size > 1000)
      this.observerIds.delete(this.observerIds.values().next().value ?? "");
    if (
      envelope.kind === "batch" &&
      envelope.payload &&
      typeof envelope.payload === "object"
    ) {
      const events = (envelope.payload as { events?: ObserverEnvelope[] })
        .events;
      if (Array.isArray(events))
        events.slice(-100).forEach((item, index) => {
          this.observe(`${eventId}:${index}`, agent, item, depth + 1);
        });
      return;
    }
    if (
      !envelope.channelId ||
      !envelope.turnId ||
      !this.channels.get(envelope.channelId)?.joined
    )
      return;
    const timestamp = Date.parse(envelope.timestamp);
    if (!Number.isFinite(timestamp) || !Number.isSafeInteger(envelope.seq))
      return;
    // ACP start/completion frames omit sessionId; turn UUID binds their lifecycle.
    const key = `${agent}:${envelope.channelId}:${envelope.turnId}`;
    const prior = this.activity.get(key);
    if (
      prior &&
      (timestamp < prior.updatedAt ||
        (timestamp === prior.updatedAt && envelope.seq <= prior.seq))
    )
      return;
    const payload = envelope.payload as {
      params?: {
        update?: { sessionUpdate?: string; title?: string; status?: string };
      };
      error?: string;
    } | null;
    const update = payload?.params?.update;
    const state =
      prior?.state === "finished" || prior?.state === "failed"
        ? prior.state
        : envelope.kind === "turn_completed" ||
            envelope.kind === "turn_finished"
          ? "finished"
          : envelope.kind === "turn_failed"
            ? "failed"
            : "working";
    const detail =
      (typeof update?.title === "string" ? update.title : "") ||
      (update?.sessionUpdate === "agent_message_chunk"
        ? "Writing a response"
        : update?.sessionUpdate === "agent_thought_chunk"
          ? "Thinking"
          : envelope.kind.replaceAll("_", " "));
    this.activity.delete(key);
    this.activity.set(key, {
      key,
      agent,
      channelId: envelope.channelId,
      sessionId:
        typeof envelope.sessionId === "string"
          ? envelope.sessionId
          : (prior?.sessionId ?? "pending"),
      turnId: envelope.turnId,
      state,
      detail,
      updatedAt: timestamp,
      seq: envelope.seq,
    });
    if (this.activity.size > 100)
      this.activity.delete(this.activity.keys().next().value ?? "");
  }

  /** Send an immutable snapshot. A late result only updates its original view. */
  async send(
    view: Conversation,
    transport: Transport,
    retry = false,
  ): Promise<void> {
    if (view.sending) return;
    if (this.connection !== "connected")
      throw new Error("Reconnect before sending. Your draft is safe.");
    const channel = this.channels.get(view.channelId);
    if (
      !channel?.joined ||
      !view.recipient ||
      !channel.members.includes(view.recipient)
    )
      throw new Error(
        "Choose a current channel member with Ctrl+R before sending.",
      );
    if (view.pending && !retry)
      throw new Error(
        "Delivery is unresolved. Use /retry to resend the identical event.",
      );
    if (!retry && !view.draft.trim()) return;
    if (Buffer.byteLength(view.draft) > 32 * 1024)
      throw new Error(
        "Message exceeds 32 KiB. Shorten the draft before sending.",
      );
    const request = retry
      ? view.pending
      : {
          requestKey: randomUUID(),
          channelId: view.channelId,
          rootEventId: view.rootEventId,
          content: view.draft,
          recipientPubkeys: [view.recipient],
        };
    if (!request) throw new Error("There is no pending message to retry.");
    if (!request.recipientPubkeys.every((key) => channel.members.includes(key)))
      throw new Error(
        "The original recipient has left. Delivery is unresolved; inspect the conversation before discarding it.",
      );
    view.pending = request;
    view.sending = true;
    view.error = undefined;
    this.changed();
    try {
      const result = (await transport.request("sendMessage", request)) as {
        accepted: boolean;
        event: RelayEvent;
      };
      if (!result.accepted)
        throw new HostError("rejected", "Relay rejected the message.");
      if (view.draft === request.content) view.draft = "";
      view.pending = undefined;
      this.event(result.event, view.key);
    } catch (error) {
      view.error = error instanceof Error ? error.message : "Send failed.";
      if (
        error instanceof HostError &&
        !["uncertain", "timeout"].includes(error.code)
      )
        view.pending = undefined;
    } finally {
      view.sending = false;
      this.changed();
    }
  }
}
