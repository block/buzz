import { Channel, invoke } from "@tauri-apps/api/core";
import {
  createAuthEvent,
  getRelayWsUrl,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { PresenceStatus, RelayEvent } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_TYPING_INDICATOR,
  KIND_USER_STATUS,
  CHANNEL_EVENT_KINDS,
  KIND_CHANNEL_THREAD_SUMMARY,
} from "@/shared/constants/kinds";
import {
  getTextPayload,
  type ConnectionState,
  type PendingEvent,
  type RelaySubscription,
  type RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import {
  buildChannelAuxDeletionFilter,
  buildChannelFilter,
  buildChannelHistoryFilter,
  buildChannelMentionFilter,
  buildGlobalStreamFilter,
} from "@/shared/api/relayChannelFilters";
import {
  clearClosedRetry,
  handleRelayClosed,
  handleSubscriptionEose,
  prepareSubscriptionEvent,
} from "@/shared/api/relayClosedRecovery";
import { replayLiveSubscriptions } from "@/shared/api/relayReconnectReplay";
import {
  activateRateLimit,
  parseRateLimitHint,
  waitForRateLimit,
} from "@/shared/api/relayRateLimitGate";
import {
  fetchChunkedHistory,
  requestFirstEventGated,
  requestHistoryGated,
} from "@/shared/api/relayGateBoundary";
import { RelayConnectionStateEmitter } from "@/shared/api/relayConnectionStateEmitter";
import {
  isServiceRestartClose,
  isWebSocketClose,
  isWebSocketError,
  shouldRefuseConnect,
  shouldScheduleReconnect,
  shouldWaitForScheduledReconnect,
} from "@/shared/api/relayReconnectPolicy";
import { RelayReconnectWaiters } from "@/shared/api/relayReconnectWaiters";
import { RelayStallWatchdog } from "@/shared/api/relayStallWatchdog";
import {
  AUTH_TIMEOUT_MS,
  BACKOFF_RESET_STABLE_MS,
  EVENT_BATCH_MS,
  HISTORY_TIMEOUT_MS,
  PUBLISH_TIMEOUT_MS,
  RECONNECT_BASE_DELAY_MS,
  RECONNECT_MAX_DELAY_MS,
  STALL_CHECK_INTERVAL_MS,
  STALL_IDLE_TIMEOUT_MS,
} from "@/shared/api/relayClientTimings";
import { closeWebSocket } from "@/shared/api/relayWebSocketClose";
import { AuthOkTracker } from "@/shared/api/relayAuthPolicy";
import { buildThreadReferenceTags } from "@/features/messages/lib/threading";

export class RelayClient {
  private wsId: number | null = null;
  private relayUrl: string | null = null;
  private connectPromise: Promise<void> | null = null;
  private reconnectTimeout: number | null = null;
  private reconnectWaiters = new RelayReconnectWaiters();
  private reconnectDelayMs = RECONNECT_BASE_DELAY_MS;
  private keepAliveRequested = false;
  private authRequest: {
    pendingEventId: string;
    resolve: () => void;
    reject: (error: Error) => void;
    timeout: number;
  } | null = null;
  private subscriptions = new Map<string, RelaySubscription>();
  private pendingEvents = new Map<string, PendingEvent>();
  private eventBuffer: Array<{ subId: string; event: RelayEvent }> = [];
  private flushTimeout: number | null = null;
  private reconnectListeners = new Set<() => void>();
  private hasConnectedOnce = false;
  private notifyReconnectListeners = false;
  private onMessageChannel: Channel<unknown> | null = null;
  private connectionGeneration = 0;
  private stabilityTimer: number | null = null;
  private visibleChannelId: string | null = null;
  private authOkTracker = new AuthOkTracker();

  private terminal = false;

  private connectionStateEmitter = new RelayConnectionStateEmitter("idle");
  private stallWatchdog = new RelayStallWatchdog({
    intervalMs: STALL_CHECK_INTERVAL_MS,
    idleTimeoutMs: STALL_IDLE_TIMEOUT_MS,
    onStall: (error) => {
      this.connectionStateEmitter.set("stalled");
      this.resetConnection(error);
    },
  });

  setVisibleChannelId(id: string | null) {
    this.visibleChannelId = id;
  }

  disconnect() {
    const error = new Error("Relay disconnected for community switch.");

    if (this.reconnectTimeout) {
      window.clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }
    if (this.stabilityTimer !== null) {
      window.clearTimeout(this.stabilityTimer);
      this.stabilityTimer = null;
    }
    this.stallWatchdog.stop();
    this.connectionGeneration++;
    this.keepAliveRequested = false;
    this.relayUrl = null;
    this.hasConnectedOnce = false;
    this.notifyReconnectListeners = false;
    this.terminal = false;
    this.visibleChannelId = null;
    this.authOkTracker.reset();
    this.connectionStateEmitter.set("idle");

    if (this.wsId !== null) {
      void closeWebSocket(this.wsId, "community switch");
      this.wsId = null;
    }

    this.connectPromise = null;
    this.reconnectWaiters.settle(error);

    if (this.authRequest) {
      window.clearTimeout(this.authRequest.timeout);
      this.authRequest.reject(error);
      this.authRequest = null;
    }

    for (const [subId, sub] of this.subscriptions) {
      if (sub.mode !== "live") {
        window.clearTimeout(sub.timeout);
        sub.reject(error);
      } else {
        clearClosedRetry(sub);
      }
      this.subscriptions.delete(subId);
    }

    for (const [eventId, pending] of this.pendingEvents) {
      window.clearTimeout(pending.timeout);
      pending.reject(error);
      this.pendingEvents.delete(eventId);
    }

    if (this.flushTimeout !== null) {
      window.clearTimeout(this.flushTimeout);
      this.flushTimeout = null;
    }
    this.eventBuffer = [];
    this.reconnectListeners.clear();
    this.connectionStateEmitter.clear();
    this.onMessageChannel = null;
    this.reconnectDelayMs = RECONNECT_BASE_DELAY_MS;
  }

  async fetchChannelHistory(channelId: string, limit = 50) {
    return this.fetchHistory(buildChannelHistoryFilter(channelId, limit));
  }

  async fetchChannelHistoryBefore(
    channelId: string,
    before: number,
    limit = 50,
  ) {
    return this.fetchHistory(
      buildChannelHistoryFilter(channelId, limit, before),
    );
  }

  async fetchAuxEventsByReference(
    channelId: string,
    referencedEventIds: string[],
    buildFilter: (
      channelId: string,
      eventIds: string[],
    ) => RelaySubscriptionFilter,
  ) {
    return fetchChunkedHistory(
      referencedEventIds,
      (eventIds) => buildFilter(channelId, eventIds),
      (filter) => this.fetchHistory(filter),
    );
  }

  async fetchAuxDeletionEventsForAuxEvents(
    channelId: string,
    auxEventIds: string[],
  ): Promise<RelayEvent[]> {
    return fetchChunkedHistory(
      auxEventIds,
      (eventIds) => buildChannelAuxDeletionFilter(channelId, eventIds),
      (filter) => this.fetchHistory(filter),
    );
  }

  async fetchEvents(filter: RelaySubscriptionFilter): Promise<RelayEvent[]> {
    return this.fetchHistory(filter);
  }

  async fetchFirstEvent(
    filter: RelaySubscriptionFilter,
  ): Promise<RelayEvent | null> {
    await this.ensureConnected();
    return requestFirstEventGated(
      this.subscriptions,
      (payload) => this.sendRaw(payload),
      (subId) => this.closeSubscription(subId),
      filter,
      HISTORY_TIMEOUT_MS,
    );
  }

  private async fetchHistory(filter: RelaySubscriptionFilter) {
    await this.ensureConnected();
    return this.requestHistory(filter);
  }

  private requestHistory(
    filter: RelaySubscriptionFilter,
  ): Promise<RelayEvent[]> {
    return requestHistoryGated(
      this.subscriptions,
      (payload) => this.sendRaw(payload),
      (subId) => this.closeSubscription(subId),
      filter,
      HISTORY_TIMEOUT_MS,
    );
  }

  async sendMessage(
    channelId: string,
    content: string,
    mentionPubkeys: string[] = [],
    extraTags: string[][] = [],
  ) {
    await this.ensureConnected();

    const tags: string[][] = [["h", channelId]];
    for (const pubkey of mentionPubkeys) {
      tags.push(["p", pubkey]);
    }
    for (const tag of extraTags) {
      tags.push(tag);
    }

    const event = await signRelayEvent({
      kind: KIND_STREAM_MESSAGE,
      content: content.trim(),
      tags,
    });

    return this.publishEvent(
      event,
      "Timed out while sending the message.",
      "Failed to send the message.",
    );
  }

  async sendPresence(status: PresenceStatus) {
    await this.ensureConnected();

    const event = await signRelayEvent({
      kind: 20001,
      content: status,
      tags: [],
    });

    return this.publishEvent(
      event,
      "Timed out while updating presence.",
      "Failed to update presence.",
    );
  }

  async sendTypingIndicator(
    channelId: string,
    parentEventId?: string | null,
    rootEventId?: string | null,
  ) {
    // Disconnected: not worth triggering a reconnect for ephemeral typing.
    if (this.wsId === null) {
      return;
    }
    const event = await signRelayEvent({
      kind: KIND_TYPING_INDICATOR,
      content: "",
      tags: buildThreadReferenceTags(
        channelId,
        parentEventId ?? null,
        rootEventId ?? null,
      ),
    });

    // Fire-and-forget: no need to wait for relay acknowledgement.
    void this.sendRaw(["EVENT", event]).catch(() => {});
  }

  async subscribeToChannel(
    channelId: string,
    onEvent: (event: RelayEvent) => void,
  ) {
    return this.subscribe(buildChannelFilter(channelId, 50), onEvent);
  }

  /** Subscribe to channel rows and aux starting now, with no history replay. */
  async subscribeToChannelLive(
    channelId: string,
    onEvent: (event: RelayEvent) => void,
  ) {
    // 39005 rides only this window-store subscription — CHANNEL_EVENT_KINDS'
    // other consumers (unread tracking, cache merges) must never see
    // summary overlays.
    return this.subscribe(
      {
        kinds: [...CHANNEL_EVENT_KINDS, KIND_CHANNEL_THREAD_SUMMARY],
        "#h": [channelId],
        limit: 1000,
        since: Math.floor(Date.now() / 1_000),
      },
      onEvent,
    );
  }

  /**
   * Subscribe to huddle lifecycle events (kinds 48100–48103) for a channel,
   * so HuddleIndicator detects active huddles without being drowned out by
   * regular channel messages. Includes the last 10 historical events.
   */
  async subscribeToHuddleEvents(
    channelId: string,
    onEvent: (event: RelayEvent) => void,
  ) {
    return this.subscribe(
      {
        kinds: [48100, 48101, 48102, 48103],
        "#h": [channelId],
        limit: 100,
      },
      onEvent,
    );
  }

  async subscribeToTypingIndicators(
    channelId: string,
    onEvent: (event: RelayEvent) => void,
  ) {
    return this.subscribe(
      {
        kinds: [KIND_TYPING_INDICATOR],
        "#h": [channelId],
        limit: 10,
        since: Math.floor(Date.now() / 1_000) - 10,
      },
      onEvent,
    );
  }

  async subscribeToPresenceUpdates(onEvent: (event: RelayEvent) => void) {
    return this.subscribe({ kinds: [20001], limit: 0 }, onEvent);
  }

  async publishUserStatus(text: string, emoji: string): Promise<void> {
    await this.ensureConnected();
    const tags: string[][] = [["d", "general"]];
    if (emoji) tags.push(["emoji", emoji]);
    const event = await signRelayEvent({
      kind: KIND_USER_STATUS,
      content: text,
      tags,
    });
    await this.publishEvent(
      event,
      "Timed out publishing user status",
      "Failed to publish user status",
    );
  }

  /** Subscribe to kind:30315 user status events (live only, no backfill). */
  async subscribeToUserStatusUpdates(onEvent: (event: RelayEvent) => void) {
    return this.subscribe(
      { kinds: [KIND_USER_STATUS], "#d": ["general"], limit: 0 },
      onEvent,
    );
  }

  async subscribeToAllStreamMessages(onEvent: (event: RelayEvent) => void) {
    return this.subscribe(buildGlobalStreamFilter(50), onEvent);
  }

  async subscribeLive(
    filter: RelaySubscriptionFilter,
    onEvent: (event: RelayEvent) => void,
  ) {
    return this.subscribe(filter, onEvent);
  }

  async subscribeToChannelMentionEvents(
    channelId: string,
    pubkey: string,
    onEvent: (event: RelayEvent) => void,
  ) {
    return this.subscribe(
      buildChannelMentionFilter(channelId, pubkey, 50),
      onEvent,
    );
  }

  async preconnect() {
    // Explicit re-engagement (reconnect card / community switch): clears the
    // terminal latch and AUTH rejection streak, and bypasses backoff once.
    this.terminal = false;
    this.authOkTracker.reset();
    this.keepAliveRequested = true;
    await this.connectBypassingBackoff();
  }

  /**
   * Environment-driven resume (online/focus/visibility): bypasses a pending
   * backoff timer but preserves the terminal latch and AUTH rejection streak
   * — only `preconnect()` clears those, so resume events during repeated
   * AUTH rejection cannot defeat the consecutive-rejection cap.
   */
  async resumeReconnect() {
    if (this.terminal) return;
    await this.connectBypassingBackoff();
  }

  private async connectBypassingBackoff() {
    if (this.reconnectTimeout !== null) {
      window.clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }
    try {
      await this.ensureConnected();
      this.reconnectWaiters.settle();
    } catch (error) {
      this.reconnectWaiters.settle(
        this.normalizeRelayError(error, "Relay reconnect failed."),
      );
      throw error;
    }
  }

  subscribeToReconnects(listener: () => void) {
    this.reconnectListeners.add(listener);
    return () => {
      this.reconnectListeners.delete(listener);
    };
  }

  /** Current connection state — synchronous read. */
  getConnectionState(): ConnectionState {
    return this.connectionStateEmitter.get();
  }

  /**
   * Subscribe to connection-state transitions. The listener fires
   * immediately with the current state, so callers need no separate
   * `getConnectionState()` call to seed their UI.
   */
  subscribeToConnectionState(listener: (state: ConnectionState) => void) {
    return this.connectionStateEmitter.subscribe(listener);
  }

  private async ensureConnected() {
    if (shouldRefuseConnect({ terminal: this.terminal })) {
      // Terminal (e.g. relay rejected auth): refuse until disconnect() or
      // preconnect() clears the latch, else the reconnect-timer catch and
      // the publish/subscribe retry wrappers would race the terminal
      // "disconnected" state back to "reconnecting".
      throw new Error("Relay session is terminal; cannot reconnect.");
    }

    if (this.connectPromise) {
      return this.connectPromise;
    }

    if (this.wsId !== null) {
      return;
    }

    if (
      shouldWaitForScheduledReconnect({
        hasPendingReconnect: this.reconnectTimeout !== null,
      })
    ) {
      // The reconnect coordinator owns outage pacing. Query, publish, and
      // subscription callers must wait for its scheduled attempt instead of
      // clearing the timer and creating an immediate reconnect storm.
      return this.reconnectWaiters.wait();
    }

    const connectPromise = this.connect();
    this.connectPromise = connectPromise;

    try {
      await connectPromise;
    } finally {
      if (this.connectPromise === connectPromise) {
        this.connectPromise = null;
      }
    }
  }

  private async connect() {
    if (this.stabilityTimer !== null) {
      window.clearTimeout(this.stabilityTimer);
      this.stabilityTimer = null;
    }

    this.connectionStateEmitter.set(
      this.hasConnectedOnce ? "reconnecting" : "connecting",
    );

    const generation = ++this.connectionGeneration;
    this.onMessageChannel = new Channel<unknown>((message) => {
      void this.handleWsMessage(message, generation).catch((error) => {
        if (generation !== this.connectionGeneration) return;
        this.resetConnection(
          this.normalizeRelayError(error, "Relay connection errored."),
        );
      });
    });

    try {
      if (!this.relayUrl) {
        this.relayUrl = await getRelayWsUrl();
      }
      const wsId = await invoke<number>("plugin:websocket|connect", {
        url: this.relayUrl,
        onMessage: this.onMessageChannel,
        config: {},
      });
      if (generation !== this.connectionGeneration) {
        void closeWebSocket(wsId, "stale connection attempt");
        throw new Error("Relay connection attempt was superseded.");
      }
      this.wsId = wsId;

      await new Promise<void>((resolve, reject) => {
        const timeout = window.setTimeout(() => {
          const error = new Error("Relay authentication timed out.");
          this.authRequest = null;
          this.resetConnection(error);
          reject(error);
        }, AUTH_TIMEOUT_MS);

        this.authRequest = {
          pendingEventId: "",
          resolve,
          reject,
          timeout,
        };
      });

      this.stabilityTimer = window.setTimeout(() => {
        this.stabilityTimer = null;
        this.reconnectDelayMs = RECONNECT_BASE_DELAY_MS;
      }, BACKOFF_RESET_STABLE_MS);

      this.connectionStateEmitter.set("connected");
      await this.replayLiveSubscriptions();
      this.stallWatchdog.start();
      this.emitReconnectIfNeeded();
    } catch (error) {
      const connectionError = this.normalizeRelayError(
        error,
        "Failed to connect to relay.",
      );
      if (generation === this.connectionGeneration) {
        this.resetConnection(connectionError);
      }
      throw connectionError;
    }
  }

  private async subscribe(
    filter: RelaySubscriptionFilter,
    onEvent: (event: RelayEvent) => void,
  ) {
    await this.ensureConnected();

    const subId = `live-${crypto.randomUUID()}`;
    let resolveReady = () => {
      return;
    };
    const ready = new Promise<void>((resolve) => {
      resolveReady = () => {
        window.clearTimeout(fallbackTimeout);
        resolve();
      };
    });
    const fallbackTimeout = window.setTimeout(() => {
      resolveReady();
    }, 250);

    this.subscriptions.set(subId, {
      mode: "live",
      filter,
      onEvent,
      resolveReady,
    });

    try {
      await this.sendRawWithReconnectRetry(
        ["REQ", subId, filter],
        "Failed to restore relay subscription.",
      );
    } catch (error) {
      window.clearTimeout(fallbackTimeout);
      this.subscriptions.delete(subId);
      throw error;
    }
    await ready;

    return async () => {
      const active = this.subscriptions.get(subId);
      if (active?.mode !== "live") {
        return;
      }

      this.subscriptions.delete(subId);
      clearClosedRetry(active);
      await this.closeSubscription(subId);
    };
  }

  private async sendRaw(payload: unknown[]) {
    if (this.wsId === null) {
      throw new Error("Relay socket is not connected.");
    }

    await invoke("plugin:websocket|send", {
      id: this.wsId,
      message: {
        type: "Text",
        data: JSON.stringify(payload),
      },
    });
  }

  private normalizeRelayError(error: unknown, fallbackMessage: string) {
    return error instanceof Error ? error : new Error(fallbackMessage);
  }

  private recoverFromSocketFailure(
    error: unknown,
    fallbackMessage: string,
  ): Error {
    const normalizedError = this.normalizeRelayError(error, fallbackMessage);
    this.resetConnection(normalizedError);
    return normalizedError;
  }

  private async sendRawWithReconnectRetry(
    payload: unknown[],
    fallbackMessage: string,
  ) {
    try {
      await this.sendRaw(payload);
    } catch (error) {
      const normalizedError = this.recoverFromSocketFailure(
        error,
        fallbackMessage,
      );
      try {
        await this.ensureConnected();
        await this.sendRaw(payload);
      } catch (retryError) {
        throw this.recoverFromSocketFailure(
          retryError,
          normalizedError.message,
        );
      }
    }
  }

  private async closeSubscription(subId: string) {
    if (this.wsId === null) {
      return;
    }

    await this.sendRaw(["CLOSE", subId]);
  }

  async publishEvent(
    event: RelayEvent,
    timeoutMessage: string,
    sendErrorMessage: string,
  ) {
    // Await the gate before sending EVENT; op timeout starts after the wait.
    await waitForRateLimit();

    return new Promise<RelayEvent>((resolve, reject) => {
      const timeout = window.setTimeout(() => {
        this.pendingEvents.delete(event.id);
        reject(new Error(timeoutMessage));
      }, PUBLISH_TIMEOUT_MS);

      this.pendingEvents.set(event.id, {
        event,
        resolve,
        reject,
        timeout,
      });

      void this.sendRaw(["EVENT", event]).catch(async (error) => {
        const pendingEvent = this.pendingEvents.get(event.id);
        this.pendingEvents.delete(event.id);
        const normalizedError = this.recoverFromSocketFailure(
          error,
          sendErrorMessage,
        );

        try {
          await this.ensureConnected();
          if (!pendingEvent) {
            throw normalizedError;
          }

          this.pendingEvents.set(event.id, pendingEvent);
          await this.sendRaw(["EVENT", event]);
        } catch (retryError) {
          window.clearTimeout(timeout);
          this.pendingEvents.delete(event.id);
          reject(
            this.recoverFromSocketFailure(retryError, normalizedError.message),
          );
        }
      });
    });
  }

  private async handleWsMessage(message: unknown, generation: number) {
    if (generation !== this.connectionGeneration) return;
    this.stallWatchdog.recordInbound();

    if (isWebSocketClose(message)) {
      if (isServiceRestartClose(message))
        this.reconnectDelayMs = RECONNECT_BASE_DELAY_MS;
      this.resetConnection(new Error("Relay connection closed."));
      return;
    }
    if (isWebSocketError(message)) {
      this.resetConnection(new Error("Relay connection errored."));
      return;
    }

    const payload = getTextPayload(message);
    if (!payload) {
      return;
    }

    let data: unknown;
    try {
      data = JSON.parse(payload);
    } catch {
      return;
    }

    if (!Array.isArray(data) || data.length === 0) {
      return;
    }

    const [type, ...rest] = data;
    if (type === "AUTH" && typeof rest[0] === "string") {
      await this.handleAuthChallenge(rest[0], generation);
      return;
    }
    if (type === "EVENT" && typeof rest[0] === "string" && rest[1]) {
      this.handleEvent(rest[0], rest[1] as RelayEvent);
      return;
    }

    if (
      type === "OK" &&
      typeof rest[0] === "string" &&
      typeof rest[1] === "boolean"
    ) {
      this.handleOk(
        rest[0],
        rest[1],
        typeof rest[2] === "string" ? rest[2] : "",
      );
      return;
    }

    if (type === "EOSE" && typeof rest[0] === "string") {
      this.handleEose(rest[0]);
      return;
    }

    if (type === "CLOSED" && typeof rest[0] === "string") {
      handleRelayClosed({
        subscriptions: this.subscriptions,
        subId: rest[0],
        message: typeof rest[1] === "string" ? rest[1] : "",
        sendReq: (subId, filter) =>
          this.sendRawWithReconnectRetry(
            ["REQ", subId, filter],
            "Failed to restore relay subscription after CLOSED.",
          ),
      });
      return;
    }

    if (type === "NOTICE" && typeof rest[0] === "string") {
      const notice: string = rest[0];
      // Relay back-pressure — arm the gate until the window expires.
      if (notice.startsWith("rate-limited:")) {
        activateRateLimit(parseRateLimitHint(notice));
      }
    }
  }

  private async handleAuthChallenge(challenge: string, generation: number) {
    if (!this.relayUrl) {
      this.relayUrl = await getRelayWsUrl();
    }

    const event = await createAuthEvent({
      challenge,
      relayUrl: this.relayUrl,
    });

    if (generation !== this.connectionGeneration || !this.authRequest) {
      return;
    }

    this.authRequest.pendingEventId = event.id;
    await this.sendRaw(["AUTH", event]);
  }

  private handleEvent(subId: string, event: RelayEvent) {
    const subscription = this.subscriptions.get(subId);
    if (!subscription) {
      return;
    }

    if (subscription.mode === "first") {
      subscription.onEvent(event);
      return;
    }

    if (!prepareSubscriptionEvent(subscription, event)) return;
    this.eventBuffer.push({ subId, event });
    this.flushTimeout ??= window.setTimeout(
      () => this.flushEventBuffer(),
      EVENT_BATCH_MS,
    );
  }

  private flushEventBuffer() {
    this.flushTimeout = null;
    const buffer = this.eventBuffer;
    this.eventBuffer = [];

    // Re-lookup: subscriptions removed during batch window are intentionally skipped.
    for (const { subId, event } of buffer) {
      const subscription = this.subscriptions.get(subId);
      if (subscription?.mode === "live") {
        subscription.onEvent(event);
      }
    }
  }

  private handleEose(subId: string) {
    handleSubscriptionEose({
      subscriptions: this.subscriptions,
      subId,
      closeSubscription: (id) => this.closeSubscription(id),
    });
  }

  private handleOk(eventId: string, success: boolean, message: string) {
    if (this.authRequest && this.authRequest.pendingEventId === eventId) {
      window.clearTimeout(this.authRequest.timeout);
      const authRequest = this.authRequest;
      this.authRequest = null;

      // Decision table lives in relayAuthPolicy.ts.
      const decision = this.authOkTracker.record(success, message);
      if (decision === "authenticated") {
        authRequest.resolve();
      } else {
        const error = new Error(message || "Relay authentication rejected.");
        authRequest.reject(error);
        this.resetConnection(error, { reconnect: decision === "retry" });
      }

      return;
    }

    const pendingEvent = this.pendingEvents.get(eventId);
    if (!pendingEvent) {
      return;
    }

    window.clearTimeout(pendingEvent.timeout);
    this.pendingEvents.delete(eventId);

    if (success) {
      pendingEvent.resolve(pendingEvent.event);
    } else {
      pendingEvent.reject(new Error(message || "Relay rejected the event."));
    }
  }

  private hasLiveSubscriptions() {
    return [...this.subscriptions.values()].some((s) => s.mode === "live");
  }

  private async replayLiveSubscriptions() {
    const generation = this.connectionGeneration;
    try {
      await replayLiveSubscriptions({
        subscriptions: this.subscriptions,
        sendRaw: (payload) => this.sendRaw(payload),
        requestHistory: (filter) => this.requestHistory(filter),
        visibleChannelId: this.visibleChannelId,
        isActive: () => this.connectionGeneration === generation,
      });
    } catch (error) {
      const reconnectError =
        error instanceof Error
          ? error
          : new Error("Failed to restore relay subscriptions.");
      this.resetConnection(reconnectError);
      throw reconnectError;
    }
  }

  private scheduleReconnect() {
    if (
      !shouldScheduleReconnect({
        terminal: this.terminal,
        hasPendingReconnect: this.reconnectTimeout !== null,
        hasLiveSocket: this.wsId !== null,
        keepAliveRequested: this.keepAliveRequested,
        hasLiveSubscriptions: this.hasLiveSubscriptions(),
      })
    ) {
      return;
    }

    // ±25% jitter spreads a fleet's AUTH storms across a 50% window instead
    // of hitting the relay at the same instant.
    const jitter = this.reconnectDelayMs * (0.75 + Math.random() * 0.5);
    const delay = Math.min(jitter, RECONNECT_MAX_DELAY_MS);
    this.reconnectDelayMs = Math.min(
      this.reconnectDelayMs * 2,
      RECONNECT_MAX_DELAY_MS,
    );

    this.reconnectTimeout = window.setTimeout(() => {
      this.reconnectTimeout = null;
      void this.ensureConnected()
        .then(() => this.reconnectWaiters.settle())
        .catch((error) => {
          this.reconnectWaiters.settle(
            this.normalizeRelayError(error, "Relay reconnect failed."),
          );
          this.scheduleReconnect();
        });
    }, delay);
  }

  private emitReconnectIfNeeded() {
    const shouldNotifyReconnectListeners =
      this.hasConnectedOnce && this.notifyReconnectListeners;

    this.hasConnectedOnce = true;
    this.notifyReconnectListeners = false;

    if (!shouldNotifyReconnectListeners) {
      return;
    }

    for (const listener of this.reconnectListeners) {
      try {
        listener();
      } catch (error) {
        console.error("Failed to handle relay reconnect", error);
      }
    }
  }

  private resetConnection(
    error: Error,
    options?: {
      reconnect?: boolean;
    },
  ) {
    this.onMessageChannel = null;
    this.stallWatchdog.stop();
    this.connectionGeneration++;
    if (this.stabilityTimer !== null) {
      window.clearTimeout(this.stabilityTimer);
      this.stabilityTimer = null;
    }
    if (this.flushTimeout !== null) window.clearTimeout(this.flushTimeout);
    this.flushTimeout = null;
    this.eventBuffer = [];

    if (options?.reconnect === false) {
      this.terminal = true;
      this.connectionStateEmitter.set("disconnected");
    } else if (
      // A late retry failure racing a terminal latch must not paint
      // "reconnecting" over the terminal "disconnected" state; stall is a
      // stronger signal than a generic drop and is kept until reconnect.
      !this.terminal &&
      this.connectionStateEmitter.get() !== "stalled"
    ) {
      this.connectionStateEmitter.set("reconnecting");
    }

    if (options?.reconnect !== false && this.hasConnectedOnce) {
      this.notifyReconnectListeners = true;
    }

    if (options?.reconnect === false && this.reconnectTimeout) {
      window.clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }
    if (options?.reconnect === false) {
      this.reconnectWaiters.settle(error);
    }

    if (this.wsId !== null) {
      void closeWebSocket(this.wsId, "connection reset");
    }

    this.wsId = null;

    if (this.authRequest) {
      window.clearTimeout(this.authRequest.timeout);
      this.authRequest.reject(error);
      this.authRequest = null;
    }

    for (const [subId, subscription] of this.subscriptions) {
      if (subscription.mode !== "live") {
        window.clearTimeout(subscription.timeout);
        subscription.reject(error);
        this.subscriptions.delete(subId);
        continue;
      }

      subscription.resolveReady?.();
      subscription.resolveReady = undefined;
      clearClosedRetry(subscription);
    }

    for (const [eventId, pendingEvent] of this.pendingEvents) {
      window.clearTimeout(pendingEvent.timeout);
      pendingEvent.reject(error);
      this.pendingEvents.delete(eventId);
    }

    if (options?.reconnect !== false) {
      this.scheduleReconnect();
    }
  }
}
