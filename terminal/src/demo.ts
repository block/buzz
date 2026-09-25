import { createHash } from "node:crypto";
import type {
  Filter,
  HostMessage,
  MessageRequest,
  RelayEvent,
  Requests,
} from "./protocol.ts";
import type { Transport } from "./transport.ts";

const human = "1".repeat(64);
const relay = "2".repeat(64);
const atlas = "3".repeat(64);
const nova = "4".repeat(64);
const engineering = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const platform = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

/** Deterministic offline transport. It never connects to or writes to a relay. */
export class DemoTransport implements Transport {
  private receive?: (message: HostMessage) => void;
  private subscriptions = new Map<string, Filter[]>();
  private timers = new Set<NodeJS.Timeout>();
  private sequence = 0;
  private events: RelayEvent[] = [];
  private readonly now = Math.floor(Date.now() / 1000);

  private event(
    kind: number,
    pubkey: string,
    content: string,
    tags: string[][],
    seconds = 0,
  ): RelayEvent {
    return {
      id: createHash("sha256")
        .update(String(++this.sequence))
        .digest("hex"),
      kind,
      pubkey,
      content,
      tags,
      created_at: this.now + seconds,
      sig: "",
    };
  }

  start(receive: (message: HostMessage) => void): void {
    this.receive = receive;
    this.events = [
      this.event(39002, relay, "", [
        ["d", engineering],
        ["p", human],
        ["p", atlas],
      ]),
      this.event(39002, relay, "", [
        ["d", platform],
        ["p", human],
        ["p", nova],
      ]),
      this.event(39000, relay, "", [
        ["d", engineering],
        ["name", "engineering"],
      ]),
      this.event(39000, relay, "", [
        ["d", platform],
        ["name", "platform"],
      ]),
      this.event(0, atlas, JSON.stringify({ name: "Atlas" }), []),
      this.event(0, nova, JSON.stringify({ name: "Nova" }), []),
      this.event(
        9,
        human,
        "Review the relay reconnect behavior. Keep the changes small, and check that switching contexts cannot lose a draft.",
        [
          ["h", engineering],
          ["p", atlas],
        ],
        -300,
      ),
      this.event(
        9,
        atlas,
        "I found a race between reconnecting and restoring the active conversation.\n\n**The fix:** capture the conversation and recipient when you send, then reconcile the acknowledgement against that snapshot. A background response must never clear the current editor.\n\n```ts\nconst target = snapshotConversation();\nconst result = await relay.send(target);\nreconcile(target.key, result);\n```\n\nI’m checking reconnects and a second agent working in parallel.",
        [["h", engineering]],
        -240,
      ),
      this.event(
        9,
        human,
        "Check the build cache while Atlas reviews the relay.",
        [
          ["h", platform],
          ["p", nova],
        ],
        -180,
      ),
      this.event(
        9,
        nova,
        "The cold build is complete. I’m comparing it with a warm build now; you can keep working with Atlas.",
        [["h", platform]],
        -120,
      ),
    ];
    receive({
      type: "ready",
      protocolVersion: 1,
      pubkey: human,
      relayPubkey: relay,
      relayUrl: "demo://local",
    });
    receive({
      type: "connection",
      status: "connected",
      message: "DEMO · offline fixtures · no messages leave this terminal",
    });
    this.later(
      () =>
        this.telemetry(nova, platform, "turn_started", "Comparing warm build"),
      1000,
    );
    this.later(
      () =>
        this.telemetry(
          atlas,
          engineering,
          "turn_started",
          "Checking reconnect isolation",
        ),
      1300,
    );
  }

  private later(callback: () => void, delay: number): void {
    const timer = setTimeout(() => {
      this.timers.delete(timer);
      callback();
    }, delay);
    this.timers.add(timer);
  }

  private telemetry(
    agent: string,
    channelId: string,
    kind: string,
    title: string,
  ): void {
    this.receive?.({
      type: "observer",
      eventId: `demo-${++this.sequence}`,
      agentPubkey: agent,
      envelope: {
        kind,
        seq: this.sequence,
        timestamp: new Date().toISOString(),
        channelId,
        sessionId: `session-${agent.slice(0, 4)}`,
        turnId: `turn-${agent.slice(0, 4)}`,
        payload: { params: { update: { sessionUpdate: "tool_call", title } } },
      },
    });
  }

  private matches(event: RelayEvent, filters: Filter[]): boolean {
    return filters.some(
      (filter) =>
        filter.kinds.includes(event.kind) &&
        (!filter.authors || filter.authors.includes(event.pubkey)) &&
        (!filter.ids || filter.ids.includes(event.id)) &&
        Object.entries(filter)
          .filter(([key]) => key.startsWith("#"))
          .every(([key, values]) =>
            (values as string[]).some((value) =>
              event.tags.some(
                (tag) => tag[0] === key.slice(1) && tag[1] === value,
              ),
            ),
          ),
    );
  }

  private emit(event: RelayEvent, subscriptionId: string): void {
    this.receive?.({
      type: "event",
      subscriptionId,
      event,
      ownerPubkey: event.kind === 0 ? human : undefined,
    });
  }

  async request<K extends keyof Requests>(
    method: K,
    params: Requests[K],
  ): Promise<unknown> {
    if (method === "subscribe") {
      const { subscriptionId, filters } = params as Requests["subscribe"];
      this.subscriptions.set(subscriptionId, filters);
      for (const event of this.events)
        if (this.matches(event, filters)) this.emit(event, subscriptionId);
      this.receive?.({ type: "eose", subscriptionId });
    } else if (method === "unsubscribe")
      this.subscriptions.delete(
        (params as Requests["unsubscribe"]).subscriptionId,
      );
    else if (method === "reconnect") {
      this.receive?.({
        type: "connection",
        status: "disconnected",
        message: "Demo reconnect…",
      });
      this.later(
        () =>
          this.receive?.({
            type: "connection",
            status: "connected",
            message: "DEMO · reconnected · no network used",
          }),
        500,
      );
    } else if (method === "sendMessage") {
      const message = params as MessageRequest;
      const tags = [
        ["h", message.channelId],
        ...message.recipientPubkeys.map((key) => ["p", key]),
      ];
      if (message.rootEventId)
        tags.push(["e", message.rootEventId, "", "reply"]);
      const event = this.event(9, human, message.content, tags, this.sequence);
      this.events.push(event);
      this.later(() => {
        const reply = this.event(
          9,
          message.recipientPubkeys[0],
          "This is an offline preview. In a connected session, your agent’s reply would appear here while other contexts keep working.",
          tags.filter((tag) => tag[0] !== "p"),
          this.sequence,
        );
        this.events.push(reply);
        for (const [id, filters] of this.subscriptions)
          if (this.matches(reply, filters)) this.emit(reply, id);
      }, 1800);
      return { accepted: true, event };
    } else if (
      method === "createChannel" ||
      method === "joinChannel" ||
      method === "addMember"
    ) {
      const { channelId } = params as Requests["createChannel"];
      const prior = this.events.findLast(
        (event) =>
          event.kind === 39002 &&
          event.tags.some((tag) => tag[0] === "d" && tag[1] === channelId),
      );
      if (method !== "createChannel" && !prior)
        throw new Error("Channel not found in offline demo");
      const roster = this.event(
        39002,
        relay,
        "",
        [
          ["d", channelId],
          ["p", human],
          ...(prior?.tags.filter((tag) => tag[0] === "p" && tag[1] !== human) ??
            []),
          ...(method === "addMember"
            ? [["p", (params as Requests["addMember"]).pubkey]]
            : []),
        ],
        this.sequence,
      );
      this.events = this.events.filter((event) => event !== prior);
      this.events.push(roster);
      if (method === "createChannel")
        this.events.push(
          this.event(39000, relay, "", [
            ["d", channelId],
            ["name", `terminal-${channelId.slice(0, 8)}`],
          ]),
        );
      for (const [id, filters] of this.subscriptions)
        if (this.matches(roster, filters)) this.emit(roster, id);
      return { accepted: true };
    }
    return {};
  }

  close(): void {
    for (const timer of this.timers) clearTimeout(timer);
    this.timers.clear();
  }
}
