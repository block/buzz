import { type Channel, invoke } from "@tauri-apps/api/core";
import { getRelayWsUrl, invokeTauri } from "@/shared/api/tauri";
import { closeWebSocket } from "./relayWebSocketClose";

export type EntrySnapshot = {
  relayUrl: string;
  verdict: "required" | "error" | null;
  pending: boolean;
  upgradeUrl: string | null;
};
const empty: EntrySnapshot = {
  relayUrl: "",
  verdict: null,
  pending: false,
  upgradeUrl: null,
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
    if (!isCurrent())
      throw new Error("Relay connection attempt was superseded.");
    await this.check(resolved);
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
    let result: { required: boolean; upgradeUrl: string | null };
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
          upgradeUrl: null,
        });
      throw error;
    }
    if (epoch !== this.epoch)
      throw new Error("Community discovery was superseded");
    this.set({
      relayUrl,
      verdict: result.required ? "required" : null,
      pending: false,
      upgradeUrl: result.upgradeUrl,
    });
    if (result.required)
      throw new UpgradeRequiredError(
        "This community requires a newer Buzz app",
      );
  }
}
