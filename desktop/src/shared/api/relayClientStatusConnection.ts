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

let currentProjectionOwner: symbol | null = null;

/**
 * Owns the connection-local status channel and binds scoped AUTH to the same
 * native socket. Retirement clears browser-visible status fail closed.
 */
export class RelayClientStatusConnection {
  readonly projectionChannel: Channel<CurrentProjection | null>;
  private readonly nativeSocketBinding: Promise<NativeSocketBinding | null>;
  private resolveNativeSocketBinding!: (
    binding: NativeSocketBinding | null,
  ) => void;
  private nativeSocketId: number | null = null;
  private readonly projectionOwner = Symbol("relay-client-status-connection");
  private settled = false;
  private authAttempted = false;
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
    currentProjectionOwner = this.projectionOwner;
    this.projectionChannel = createCurrentProjectionChannel(
      () =>
        currentProjectionOwner === this.projectionOwner &&
        this.nativeSocketId !== null &&
        this.isActive(this.nativeSocketId),
    );
    clearCurrentProjection();
  }

  /** Open the native socket and attach its guarded projection channel. */
  async connect(
    relayUrl: string,
    onMessage: Channel<unknown>,
  ): Promise<number> {
    return invoke<number>("plugin:websocket|connect_with_status", {
      url: relayUrl,
      onMessage,
      onProjection: this.projectionChannel,
      config: {},
    });
  }

  /** Bind the native socket identifier returned by a successful connection. */
  bind(id: number, relayUrl: string) {
    this.nativeSocketId = id;
    this.settle({ id, relayUrl });
  }

  /** Retire this owner; late native updates can no longer change projection. */
  retire() {
    this.settle(null);
    this.nativeSocketId = null;
    if (currentProjectionOwner === this.projectionOwner) {
      currentProjectionOwner = null;
      clearCurrentProjection();
    }
  }

  /** Answer one challenge with scoped AUTH when this socket is still current. */
  async handleAuthChallenge(challenge: string) {
    if (this.authAttempted) return;
    this.authAttempted = true;
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
