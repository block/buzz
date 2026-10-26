import type { ScreenShare } from "./screenShare";

type Session = {
  share: ScreenShare;
  channel: string;
  stream?: MediaStream;
  stopFrames?: () => void;
  stopPublisher?: () => void;
};

/** Capture, consent reservation, and publication operations for one camera. */
export type CameraControllerDependencies = {
  current: (channel: string) => boolean;
  reserve: (
    channel: string,
    deviceId: string,
    label: string,
  ) => Promise<ScreenShare>;
  retire: (sessionId: string, error?: string) => Promise<unknown>;
  notifyStop: (share: ScreenShare, channel: string) => Promise<unknown>;
  acquire: (deviceId: string, current: () => boolean) => Promise<MediaStream>;
  pump: (
    stream: MediaStream,
    share: ScreenShare,
    current: () => boolean,
    fail: (error: unknown) => void,
  ) => Promise<() => void>;
  publish: (
    share: ScreenShare,
    current: () => boolean,
    fail: (error: unknown) => void,
  ) => () => void;
  changed: (busy: boolean) => void;
  failed: (error: unknown) => void;
};

/** One acquisition/producer owned by the main window, with cancellable consent. */
export class CameraController {
  private generation = 0;
  private acquiring = false;
  private session: Session | null = null;
  private readonly deps: CameraControllerDependencies;
  constructor(deps: CameraControllerDependencies) {
    this.deps = deps;
  }

  /** Native mirroring may retire a reservation independently of browser capture. */
  get sessionId(): string | null {
    return this.session?.share.session_id ?? null;
  }

  /** The consent scope, not whichever room was most recently rendered. */
  get channelId(): string | null {
    return this.session?.channel ?? null;
  }

  /** Enable or replace the camera only on an explicit, current-huddle action. */
  async start(channel: string, deviceId: string, label: string) {
    if (this.acquiring)
      throw new Error("Camera permission request is already in progress");
    this.acquiring = true;
    this.deps.changed(true);
    const generation = ++this.generation;
    const current = () =>
      generation === this.generation && this.deps.current(channel);
    const previous = this.session;
    this.session = null;
    let session: Session | null = null;
    try {
      if (previous) await this.retire(previous);
      if (!current()) return;
      const share = await this.deps.reserve(channel, deviceId, label);
      if (!current()) {
        await this.deps.retire(share.session_id);
        return;
      }
      session = { share, channel };
      this.session = session;
      const stream = await this.deps.acquire(deviceId, current);
      if (!current()) {
        stream.getTracks().forEach((track) => {
          track.stop();
        });
        return;
      }
      session.stream = stream;
      const failed = (error: unknown) => {
        if (current()) void this.stop(String(error)).catch(this.deps.failed);
      };
      const stopFrames = await this.deps.pump(stream, share, current, failed);
      if (!current()) {
        stopFrames();
        stream.getTracks().forEach((track) => {
          track.stop();
        });
        return;
      }
      session.stopFrames = stopFrames;
      session.stopPublisher = this.deps.publish(share, current, failed);
    } catch (error) {
      if (current()) {
        await this.stop(String(error));
        throw error;
      }
    } finally {
      try {
        if (!current() && session && this.session === session) {
          await this.stop();
        }
      } finally {
        this.acquiring = false;
        this.deps.changed(false);
      }
    }
  }

  /** Release the LED/track before awaiting IPC; a late grant can never publish. */
  async stop(error?: string) {
    this.generation++;
    const session = this.session;
    this.session = null;
    if (!session) return;
    await this.retire(session, error);
  }

  private async retire(session: Session, error?: string) {
    session.stopFrames?.();
    session.stopPublisher?.();
    session.stream?.getTracks().forEach((track) => {
      track.stop();
    });
    await this.deps.retire(session.share.session_id, error);
    // A missing stop notification is recovered by the bounded receiver expiry.
    await this.deps.notifyStop(session.share, session.channel).catch(() => {});
  }
}
