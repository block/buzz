import type { Launch } from "./launch.ts";
import type { Filter, HostMessage } from "./protocol.ts";
import { Startup } from "./startup.ts";
import { HISTORY_LIMIT, type Store } from "./store.ts";
import type { Transport } from "./transport.ts";

/** Owns relay subscriptions for open views, not the remote agents' lifetime. */
export class Session {
  readonly store: Store;
  readonly transport: Transport;
  readonly startup?: Startup;
  private installed = new Map<string, string>();
  private pending = new Set<string>();
  private generation = 0;
  private discoveryReady = false;
  private queued = false;
  private stopped = false;
  private dispose?: () => void;

  constructor(store: Store, transport: Transport, launch?: Launch) {
    this.store = store;
    this.transport = transport;
    if (launch)
      this.startup = new Startup(launch, store, transport, () => {
        this.installed.delete("startup");
        this.scheduleSync();
      });
  }

  /** Begin receiving state and maintaining live history subscriptions. */
  start(): void {
    this.dispose = this.store.subscribe(() => this.scheduleSync());
    this.transport.start((message) => this.receive(message));
  }

  private receive(message: HostMessage): void {
    if (this.stopped) return;
    if ("type" in message && message.type === "connection") {
      this.generation++;
      this.discoveryReady = false;
      this.pending.clear();
      for (const id of this.installed.keys()) this.installed.set(id, "");
    }
    if (
      "type" in message &&
      message.type === "eose" &&
      message.subscriptionId === "discovery"
    )
      this.discoveryReady = true;
    this.store.handle(message);
    this.startup?.receive(message);
    this.startup?.reconcile();
  }

  private scheduleSync(): void {
    if (this.queued || this.stopped) return;
    this.queued = true;
    queueMicrotask(() => {
      this.queued = false;
      this.sync();
    });
  }

  private sync(): void {
    if (
      this.stopped ||
      this.store.connection !== "connected" ||
      !this.store.pubkey
    )
      return;
    const desired = new Map<string, Filter[]>([
      [
        "discovery",
        [
          {
            kinds: [39002],
            authors: [this.store.relayPubkey],
            "#p": [this.store.pubkey],
            limit: this.startup ? 99 : 100,
          },
          { kinds: [30177], authors: [this.store.pubkey], limit: 1000 },
        ],
      ],
      ["observer", [{ kinds: [24200], "#p": [this.store.pubkey], limit: 0 }]],
    ]);
    if (this.startup)
      desired.set("startup", [
        {
          kinds: [39000, 39002],
          authors: [this.store.relayPubkey],
          "#d": [this.startup.launch.channelId],
          limit: 2,
        },
      ]);
    const channels = [...this.store.channels.values()];
    if (channels.length)
      desired.set("channels", [
        {
          kinds: [39000, 39002],
          authors: [this.store.relayPubkey],
          "#d": channels.map((channel) => channel.id),
          limit: 1000,
        },
      ]);
    const people = [
      ...new Set([
        ...(this.startup?.launch.agent ? [this.startup.launch.agent] : []),
        ...this.store.agentPolicies.keys(),
        ...channels
          .filter((channel) => channel.joined)
          .flatMap((channel) => channel.members),
      ]),
    ].slice(0, 1000);
    if (people.length && (!this.startup || this.discoveryReady)) {
      const filters: Filter[] = [];
      for (let index = 0; index < people.length; index += 200)
        filters.push({
          kinds: [0],
          authors: people.slice(index, index + 200),
          limit: 200,
        });
      desired.set("profiles", filters);
    }
    for (const view of this.store.views.values()) {
      if (!this.store.channels.get(view.channelId)?.joined) continue;
      const filters: Filter[] = [
        {
          kinds: [9, 40002, 40008],
          "#h": [view.channelId],
          limit: HISTORY_LIMIT,
        },
      ];
      if (view.rootEventId) {
        filters[0]["#e"] = [view.rootEventId];
        filters.push({
          kinds: [9, 40002, 40008],
          "#h": [view.channelId],
          ids: [view.rootEventId],
          limit: 1,
        });
      }
      desired.set(view.key, filters);
    }
    for (const id of this.installed.keys()) {
      if (!desired.has(id)) {
        this.installed.delete(id);
        void this.transport
          .request("unsubscribe", { subscriptionId: id })
          .catch((error) => this.report(error));
      }
    }
    for (const [id, filters] of desired) {
      const signature = JSON.stringify(filters);
      if (this.pending.has(id) || this.installed.get(id) === signature)
        continue;
      const generation = this.generation;
      this.pending.add(id);
      this.installed.set(id, signature);
      void this.transport
        .request("subscribe", { subscriptionId: id, filters })
        .catch((error) => {
          if (generation !== this.generation || this.stopped) return;
          this.store.handle({
            type: "closed",
            subscriptionId: id,
            message:
              error instanceof Error ? error.message : "Subscription failed",
          });
        })
        .finally(() => {
          if (generation !== this.generation || this.stopped) return;
          this.pending.delete(id);
          this.scheduleSync();
        });
    }
  }

  private report(error: unknown): void {
    if (this.stopped) return;
    this.store.notice =
      error instanceof Error ? error.message : "Relay request failed";
    this.store.changed();
  }

  /** Detach from the relay without cancelling any remote turn. */
  close(): void {
    this.stopped = true;
    this.startup?.close();
    this.dispose?.();
    this.transport.close();
  }
}
