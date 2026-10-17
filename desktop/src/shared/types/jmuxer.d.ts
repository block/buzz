declare module "jmuxer" {
  /** Browser-only surface of jMuxer's documented live H.264 API. */
  export default class JMuxer {
    constructor(options: {
      node: HTMLVideoElement;
      mode: "video";
      fps: number;
      flushingTime: number;
      maxDelay: number;
      clearBuffer: boolean;
      onReady?: () => void;
      onError?: () => void;
      onUnsupportedCodec?: () => void;
    });
    feed(packet: {
      video: Uint8Array;
      duration: number;
      isLastVideoFrameComplete: boolean;
    }): void;
    destroy(): void;
  }
}
