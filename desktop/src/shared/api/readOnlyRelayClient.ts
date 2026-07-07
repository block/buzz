import { Channel, invoke } from "@tauri-apps/api/core";

import { createAuthEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  getTextPayload,
  sortEvents,
  type RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import { closeWebSocket } from "@/shared/api/relayWebSocketClose";

const AUTH_TIMEOUT_MS = 8_000;
const HISTORY_TIMEOUT_MS = 8_000;
const PUBLISH_TIMEOUT_MS = 8_000;

type PendingHistory = {
  events: RelayEvent[];
  resolve: (events: RelayEvent[]) => void;
  reject: (error: Error) => void;
  timeout: number;
};

type PendingPublish = {
  resolve: () => void;
  reject: (error: Error) => void;
  timeout: number;
};

/**
 * Minimal relay session for inactive-workspace observation (and the rail's
 * cross-relay "mark all as read" publish). It never reads or mutates the
 * active workspace backend relay URL; callers pass an explicit URL and should
 * disconnect as soon as their polling batch finishes.
 */
export class ReadOnlyRelayClient {
  private wsId: number | null = null;
  private onMessageChannel: Channel<unknown> | null = null;
  private connectPromise: Promise<void> | null = null;
  private authRequest: {
    pendingEventId: string;
    resolve: () => void;
    reject: (error: Error) => void;
    timeout: number;
  } | null = null;
  private histories = new Map<string, PendingHistory>();
  private publishes = new Map<string, PendingPublish>();
  private generation = 0;

  private readonly relayUrl: string;

  constructor(relayUrl: string) {
    this.relayUrl = relayUrl;
  }

  async connect(): Promise<void> {
    if (this.wsId !== null) return;
    if (this.connectPromise) return this.connectPromise;

    const promise = this.openConnection();
    this.connectPromise = promise;
    try {
      await promise;
    } finally {
      if (this.connectPromise === promise) {
        this.connectPromise = null;
      }
    }
  }

  disconnect(): void {
    const error = new Error("Read-only relay observer disconnected.");
    this.generation++;

    if (this.wsId !== null) {
      void closeWebSocket(this.wsId, "observer disconnected");
      this.wsId = null;
    }

    if (this.authRequest) {
      window.clearTimeout(this.authRequest.timeout);
      this.authRequest.reject(error);
      this.authRequest = null;
    }

    for (const [subId, pending] of this.histories) {
      window.clearTimeout(pending.timeout);
      pending.reject(error);
      this.histories.delete(subId);
    }

    for (const [eventId, pending] of this.publishes) {
      window.clearTimeout(pending.timeout);
      pending.reject(error);
      this.publishes.delete(eventId);
    }

    this.onMessageChannel = null;
    this.connectPromise = null;
  }

  async fetchEvents(filter: RelaySubscriptionFilter): Promise<RelayEvent[]> {
    await this.connect();
    return this.requestHistory(filter);
  }

  async publishEvent(event: RelayEvent): Promise<void> {
    await this.connect();
    if (this.wsId === null) {
      throw new Error("Read-only relay socket is not connected.");
    }

    return new Promise<void>((resolve, reject) => {
      const timeout = window.setTimeout(() => {
        this.publishes.delete(event.id);
        reject(new Error("Timed out publishing to observer relay."));
      }, PUBLISH_TIMEOUT_MS);

      this.publishes.set(event.id, { resolve, reject, timeout });

      void this.sendRaw(["EVENT", event]).catch((error) => {
        window.clearTimeout(timeout);
        this.publishes.delete(event.id);
        reject(
          error instanceof Error
            ? error
            : new Error("Failed to publish to observer relay."),
        );
      });
    });
  }

  private async openConnection(): Promise<void> {
    const generation = ++this.generation;
    this.onMessageChannel = new Channel<unknown>((message) => {
      void this.handleWsMessage(message, generation);
    });

    this.wsId = await invoke<number>("plugin:websocket|connect", {
      url: this.relayUrl,
      onMessage: this.onMessageChannel,
      config: {},
    });

    await new Promise<void>((resolve, reject) => {
      const timeout = window.setTimeout(() => {
        this.authRequest = null;
        this.disconnect();
        reject(new Error("Timed out while authenticating observer relay."));
      }, AUTH_TIMEOUT_MS);

      this.authRequest = {
        pendingEventId: "",
        resolve,
        reject,
        timeout,
      };
    });
  }

  private requestHistory(
    filter: RelaySubscriptionFilter,
  ): Promise<RelayEvent[]> {
    if (this.wsId === null) {
      return Promise.reject(
        new Error("Read-only relay socket is not connected."),
      );
    }

    return new Promise<RelayEvent[]>((resolve, reject) => {
      const subId = `observer-history-${crypto.randomUUID()}`;
      const timeout = window.setTimeout(() => {
        this.histories.delete(subId);
        void this.sendRaw(["CLOSE", subId]).catch(() => {});
        reject(new Error("Timed out while loading observer relay history."));
      }, HISTORY_TIMEOUT_MS);

      this.histories.set(subId, { events: [], resolve, reject, timeout });

      void this.sendRaw(["REQ", subId, filter]).catch((error) => {
        window.clearTimeout(timeout);
        this.histories.delete(subId);
        reject(
          error instanceof Error
            ? error
            : new Error("Failed to request observer relay history."),
        );
      });
    });
  }

  private async sendRaw(payload: unknown[]): Promise<void> {
    if (this.wsId === null) {
      throw new Error("Read-only relay socket is not connected.");
    }

    await invoke("plugin:websocket|send", {
      id: this.wsId,
      message: {
        type: "Text",
        data: JSON.stringify(payload),
      },
    });
  }

  private async handleWsMessage(
    message: unknown,
    generation: number,
  ): Promise<void> {
    if (generation !== this.generation) return;

    if (
      typeof message === "object" &&
      message !== null &&
      "type" in message &&
      (message.type === "Close" || message.type === "Error")
    ) {
      this.disconnect();
      return;
    }

    const payload = getTextPayload(message);
    if (!payload) return;

    let data: unknown;
    try {
      data = JSON.parse(payload);
    } catch {
      return;
    }
    if (!Array.isArray(data) || data.length === 0) return;

    const [type, ...rest] = data;
    if (type === "AUTH" && typeof rest[0] === "string") {
      await this.handleAuthChallenge(rest[0], generation);
      return;
    }
    if (type === "EVENT" && typeof rest[0] === "string" && rest[1]) {
      this.histories.get(rest[0])?.events.push(rest[1] as RelayEvent);
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
    }
  }

  private async handleAuthChallenge(
    challenge: string,
    generation: number,
  ): Promise<void> {
    const event = await createAuthEvent({
      challenge,
      relayUrl: this.relayUrl,
    });

    if (generation !== this.generation || !this.authRequest) return;
    this.authRequest.pendingEventId = event.id;
    await this.sendRaw(["AUTH", event]);
  }

  private handleOk(eventId: string, success: boolean, message: string): void {
    const publish = this.publishes.get(eventId);
    if (publish) {
      window.clearTimeout(publish.timeout);
      this.publishes.delete(eventId);
      if (success) {
        publish.resolve();
      } else {
        publish.reject(
          new Error(message || "Observer relay rejected the event."),
        );
      }
      return;
    }

    if (!this.authRequest || this.authRequest.pendingEventId !== eventId) {
      return;
    }

    window.clearTimeout(this.authRequest.timeout);
    const authRequest = this.authRequest;
    this.authRequest = null;

    if (success) {
      authRequest.resolve();
      return;
    }

    authRequest.reject(
      message
        ? new Error(message)
        : new Error("Observer relay authentication rejected."),
    );
    this.disconnect();
  }

  private handleEose(subId: string): void {
    const pending = this.histories.get(subId);
    if (!pending) return;

    window.clearTimeout(pending.timeout);
    this.histories.delete(subId);
    void this.sendRaw(["CLOSE", subId]).catch(() => {});
    pending.resolve(sortEvents(pending.events));
  }
}

export async function withReadOnlyRelayClient<T>(
  relayUrl: string,
  callback: (client: ReadOnlyRelayClient) => Promise<T>,
): Promise<T> {
  const client = new ReadOnlyRelayClient(relayUrl);
  try {
    return await callback(client);
  } finally {
    client.disconnect();
  }
}
