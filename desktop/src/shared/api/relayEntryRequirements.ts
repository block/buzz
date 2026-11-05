import { type Channel, invoke } from "@tauri-apps/api/core";
import { getRelayWsUrl, invokeTauri } from "@/shared/api/tauri";
import { closeWebSocket } from "./relayWebSocketClose";

export type EntrySnapshot = {
  relayUrl: string;
  verdict: "required" | "error" | null;
  pending: boolean;
};
const empty: EntrySnapshot = {
  relayUrl: "",
  verdict: null,
  pending: false,
};
class UpgradeRequiredError extends Error {}

/** Owned by the relay session: never persisted, invalidated on community switch. */
export class RelayEntryRequirements {
  static isUpgradeRequired(error: unknown) {
    return error instanceof UpgradeRequiredError;
  }
  private epoch = 0;
  private snapshot = empty;
  private listeners = new Set<() => void>();
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private set(snapshot: EntrySnapshot) {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener();
  }
  reset() {
    this.epoch++;
    this.set(empty);
  }
  async connect(
    relayUrl: string | null,
    onMessage: Channel<unknown>,
    isCurrent: () => boolean,
  ) {
    const resolved = relayUrl ?? (await getRelayWsUrl());
    const address = new URL(resolved);
    if (
      !["ws:", "wss:"].includes(address.protocol) ||
      address.username ||
      address.password
    ) {
      throw new Error("Community URL must use ws or wss without credentials");
    }
    if (!isCurrent())
      throw new Error("Relay connection attempt was superseded.");
    try {
      await this.check(resolved);
    } catch (error) {
      if (RelayEntryRequirements.isUpgradeRequired(error)) throw error;
      // Unknown policy is not optional. The relay still enforces admission;
      // a missing public document must not break legacy/free connections.
      console.warn(
        "Could not check community authentication requirements; trying the relay",
        error,
      );
    }
    if (!isCurrent())
      throw new Error("Relay connection attempt was superseded.");
    const wsId = await invoke<number>("plugin:websocket|connect", {
      url: resolved,
      onMessage,
      config: {},
    });
    if (!isCurrent()) {
      void closeWebSocket(wsId, "stale connection attempt");
      throw new Error("Relay connection attempt was superseded.");
    }
    return { relayUrl: resolved, wsId };
  }
  async check(relayUrl: string) {
    const epoch = ++this.epoch;
    this.set({
      ...(this.snapshot.relayUrl === relayUrl ? this.snapshot : empty),
      relayUrl,
      pending: true,
    });
    let result: {
      required: boolean;
    };
    try {
      result = await invokeTauri("fetch_relay_entry_requirements", {
        relayUrl,
      });
      if (!result || typeof result.required !== "boolean")
        throw new Error("Invalid community discovery response");
    } catch (error) {
      if (epoch === this.epoch)
        this.set({
          relayUrl,
          verdict: "error",
          pending: false,
        });
      throw error;
    }
    if (epoch !== this.epoch)
      throw new Error("Community discovery was superseded");
    this.set({
      relayUrl,
      verdict: result.required ? "required" : null,
      pending: false,
    });
    if (result.required)
      throw new UpgradeRequiredError(
        "This community requires a newer Buzz app",
      );
  }
}
