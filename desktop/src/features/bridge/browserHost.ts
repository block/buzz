// Slice R desktop glue: remote browser sessions are pinned sites added by a confirmed "Open on desktop" command.
import { closePinWebview, screenshotPinWebview } from "@/features/pinned-sites/lib/pinWebview";
import type { PinnedSiteDraft } from "@/features/pinned-sites/lib/types";
import type { BrowserDeps } from "./commandHandler";
import { BrowserSessionManager, fitImage, type Encoded } from "./browserSessions";

export const remoteBrowserManager = new BrowserSessionManager();

export type BrowserHost = {
  savePin(input: { draft: PinnedSiteDraft }): Promise<{ id: string; name: string }>;
  deletePin(pin: { id: string; name: string }): Promise<void>;
  goPin(pinId: string): void;
};

async function encodeJpeg(png: Uint8Array, maxW: number, quality: number): Promise<Encoded> {
  const bmp = await createImageBitmap(new Blob([png as BlobPart], { type: "image/png" }));
  const scale = Math.min(1, maxW / bmp.width);
  const w = Math.max(1, Math.round(bmp.width * scale));
  const h = Math.max(1, Math.round(bmp.height * scale));
  const canvas = new OffscreenCanvas(w, h);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no canvas");
  ctx.drawImage(bmp, 0, 0, w, h);
  bmp.close();
  const blob = await canvas.convertToBlob({ type: "image/jpeg", quality });
  return { bytes: new Uint8Array(await blob.arrayBuffer()), w, h };
}

/** Set while the publisher is mounted: ends every remotely visible session (desktop "Stop sharing" button). */
export let stopAllRemoteBrowserSessions: () => Promise<void> = async () => {};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export function makeBrowserDeps(
  host: () => BrowserHost,
  publishShot: BrowserDeps["publishShot"],
  publishSessions: () => Promise<void>,
): BrowserDeps {
  const pins = new Map<string, { id: string; name: string }>();
  const deps: BrowserDeps = {
    manager: remoteBrowserManager,
    publishShot,
    publishSessions,
    async open(url, hostname) {
      const pin = await host().savePin({
        draft: { name: hostname, url, icon: "globe", pollForChanges: false, openMatchingLinks: false, community: false },
      });
      pins.set(pin.id, pin);
      host().goPin(pin.id);
      return { sessionId: pin.id, title: pin.name };
    },
    async close(sessionId) {
      await closePinWebview(sessionId);
      const pin = pins.get(sessionId);
      if (pin) await host().deletePin(pin);
      pins.delete(sessionId);
    },
    async capture(sessionId) {
      // The pin's webview is only live while its screen is showing, so bring it forward first.
      host().goPin(sessionId);
      await sleep(1200);
      const r = await screenshotPinWebview(sessionId);
      if (!r.bytes?.length) throw new Error("The page isn't open on the desktop right now.");
      const png = Uint8Array.from(r.bytes);
      const fit = await fitImage((w, q) => encodeJpeg(png, w, q));
      if (!fit) throw new Error("The screenshot is too large to send.");
      return { mime: "image/jpeg", w: fit.w, h: fit.h, bytes: fit.bytes };
    },
  };
  stopAllRemoteBrowserSessions = async () => {
    for (const s of remoteBrowserManager.list()) {
      try {
        await deps.close(s.sessionId);
      } finally {
        remoteBrowserManager.remove(s.sessionId);
      }
    }
    await publishSessions();
  };
  return deps;
}
