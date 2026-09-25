import type { Launch } from "./launch.ts";
import type { HostMessage } from "./protocol.ts";
import type { Conversation, Store } from "./store.ts";
import type { Transport } from "./transport.ts";

/** Creates or opens exactly the requested channel; reconnect never creates another. */
export class Startup {
  readonly launch: Launch;
  private readonly store: Store;
  private readonly transport: Transport;
  private readonly refreshRoster: () => void;
  private attempted = false;
  private busy = false;
  private opened = false;
  private stopped = false;
  private discovering = true;
  private profilesReady = false;
  private autoSelected = false;
  private selectedAgent?: string;
  private adding = false;
  private awaitingMembership = false;

  constructor(
    launch: Launch,
    store: Store,
    transport: Transport,
    refreshRoster: () => void,
  ) {
    this.launch = launch;
    this.store = store;
    this.transport = transport;
    this.refreshRoster = refreshRoster;
  }

  /** Consume authenticated roster/profile notifications after the store reduces them. */
  receive(message: HostMessage): void {
    if (this.stopped || !("type" in message)) return;
    if (message.type === "eose") {
      if (message.subscriptionId === "discovery") this.discovering = false;
      if (message.subscriptionId === "profiles") this.profilesReady = true;
    }
    const channel = this.store.channels.get(this.launch.channelId);
    if (channel?.joined && !this.opened) {
      this.opened = true;
      this.store.open(channel.id);
      this.store.notice = `Resume with buzz join ${channel.id}`;
      this.store.changed();
    }
    const view = this.store.views.get(`${this.launch.channelId}:`);
    if (view && !view.recipient && !this.selectedAgent && !this.autoSelected) {
      const owned = [...this.store.profiles.values()].filter(
        (profile) =>
          profile.owner === this.store.pubkey &&
          (this.launch.mode === "new" ||
            channel?.members.includes(profile.pubkey)),
      );
      const agent =
        this.launch.agent ??
        (!this.discovering && this.profilesReady && owned.length === 1
          ? owned[0].pubkey
          : undefined);
      if (agent) {
        this.autoSelected = true;
        void this.selectAgent(view, agent).catch((error) => this.report(error));
      }
    }
    if (
      !this.attempted &&
      this.store.connection === "connected" &&
      (this.launch.mode === "new" ||
        (message.type === "eose" && message.subscriptionId === "startup"))
    )
      void this.retry().catch((error) => this.report(error));
  }

  /** Retry the same channel operation after a rejection or uncertain acknowledgement. */
  async retry(): Promise<void> {
    if (this.busy || this.stopped) return;
    if (this.store.connection !== "connected")
      throw new Error("Reconnect before retrying channel setup.");
    this.attempted = true;
    this.busy = true;
    try {
      if (!this.store.channels.get(this.launch.channelId)?.joined) {
        await this.transport.request(
          this.launch.mode === "new" ? "createChannel" : "joinChannel",
          {
            requestKey: `startup:${this.launch.channelId}`,
            channelId: this.launch.channelId,
          },
        );
      }
      if (this.stopped) return;
      this.refreshRoster();
      const view = this.store.views.get(`${this.launch.channelId}:`);
      if (view && this.selectedAgent && !view.recipient && !this.adding)
        await this.selectAgent(view, this.selectedAgent);
    } finally {
      this.busy = false;
    }
  }

  /** Attach an exact existing agent to a newly created channel, then await its roster. */
  async selectAgent(view: Conversation, pubkey: string): Promise<void> {
    if (view.pending || view.sending)
      throw new Error(
        "Resolve the pending send before changing its recipient.",
      );
    if (this.adding)
      throw new Error(
        "Agent membership is still being confirmed. Use /retry-setup if it fails.",
      );
    if (this.store.channels.get(view.channelId)?.members.includes(pubkey)) {
      this.selectedAgent = pubkey;
      this.awaitingMembership = false;
      this.store.selectRecipient(view, pubkey);
      return;
    }
    if (this.launch.mode !== "new" || view.channelId !== this.launch.channelId)
      throw new Error(
        "This agent is not a member of the channel. Invite it before selecting it.",
      );
    if (
      this.selectedAgent &&
      this.selectedAgent !== pubkey &&
      !this.store.channels
        .get(view.channelId)
        ?.members.includes(this.selectedAgent)
    )
      throw new Error(
        "Resolve the current agent invitation with /retry-setup first.",
      );
    this.selectedAgent = pubkey;
    this.adding = true;
    this.awaitingMembership = true;
    view.recipient = undefined;
    this.store.changed();
    try {
      await this.transport.request("addMember", {
        requestKey: `agent:${view.channelId}:${pubkey}`,
        channelId: view.channelId,
        pubkey,
      });
      if (!this.stopped) this.refreshRoster();
    } finally {
      this.adding = false;
    }
    this.reconcile();
  }

  /** Select only after the relay confirms membership, including late acknowledgements. */
  reconcile(): void {
    if (this.stopped || !this.awaitingMembership) return;
    const view = this.store.views.get(`${this.launch.channelId}:`);
    if (
      view &&
      !view.recipient &&
      !view.pending &&
      !view.sending &&
      this.selectedAgent &&
      this.store.channels
        .get(view.channelId)
        ?.members.includes(this.selectedAgent)
    ) {
      this.awaitingMembership = false;
      this.store.selectRecipient(view, this.selectedAgent);
    }
  }

  private report(error: unknown): void {
    if (this.stopped) return;
    this.store.notice = `${error instanceof Error ? error.message : "Channel setup failed"}. Retry with /retry-setup; resume with buzz join ${this.launch.channelId}`;
    this.store.changed();
  }

  /** Ignore late setup completions after the terminal detaches. */
  close(): void {
    this.stopped = true;
  }
}
