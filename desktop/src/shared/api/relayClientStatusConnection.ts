import { type Channel, invoke } from "@tauri-apps/api/core";
import { createAuthEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  clearCurrentProjection,
  createCurrentProjectionChannel,
  type CurrentProjection,
} from "@/features/binding-status/currentProjectionStore";

type NativeSocketBinding = Readonly<{
  id: number;
  relayUrl: string;
}>;

export class RelayClientStatusConnection {
  readonly projectionChannel: Channel<CurrentProjection | null>;
  private readonly nativeSocketBinding: Promise<NativeSocketBinding | null>;
  private resolveNativeSocketBinding!: (
    binding: NativeSocketBinding | null,
  ) => void;
  private nativeSocketId: number | null = null;
  private settled = false;
  private readonly isActive: (nativeSocketId: number) => boolean;
  private readonly isAuthActive: (nativeSocketId: number) => boolean;
  private readonly setPendingEventId: (eventId: string) => void;
  private readonly sendAuth: (event: RelayEvent) => Promise<void>;

  constructor(
    isActive: (nativeSocketId: number) => boolean,
    isAuthActive: (nativeSocketId: number) => boolean,
    setPendingEventId: (eventId: string) => void,
    sendAuth: (event: RelayEvent) => Promise<void>,
  ) {
    this.isActive = isActive;
    this.isAuthActive = isAuthActive;
    this.setPendingEventId = setPendingEventId;
    this.sendAuth = sendAuth;
    this.nativeSocketBinding = new Promise((resolve) => {
      this.resolveNativeSocketBinding = resolve;
    });
    this.projectionChannel = createCurrentProjectionChannel(
      () => this.nativeSocketId !== null && this.isActive(this.nativeSocketId),
    );
    clearCurrentProjection();
  }

  async connect(
    relayUrl: string,
    onMessage: Channel<unknown>,
  ): Promise<number> {
    const id = await invoke<number>("plugin:websocket|connect_with_status", {
      url: relayUrl,
      onMessage,
      onProjection: this.projectionChannel,
      config: {},
    });
    return id;
  }

  bind(id: number, relayUrl: string) {
    this.nativeSocketId = id;
    this.settle({ id, relayUrl });
  }

  retire() {
    this.settle(null);
    this.nativeSocketId = null;
    clearCurrentProjection();
  }

  async handleAuthChallenge(challenge: string) {
    const binding = await this.nativeSocketBinding;
    if (!binding) return;
    const event = await createAuthEvent({
      challenge,
      nativeWebsocketId: binding.id,
      relayUrl: binding.relayUrl,
    });
    if (!this.isAuthActive(binding.id)) return;
    this.setPendingEventId(event.id);
    await this.sendAuth(event);
  }

  private settle(binding: NativeSocketBinding | null) {
    if (this.settled) return;
    this.settled = true;
    this.resolveNativeSocketBinding(binding);
  }
}
