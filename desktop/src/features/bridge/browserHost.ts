// Slice R desktop glue: the bridge's browsers ARE the desktop's Browsers page (playground sessions). A browser added from the
// web is a normal playground browser (so it shows on the desktop Browsers page); the desktop's full list is published back.
import { clearBrowserAgentGrant, listBrowserAgentGrants, setBrowserAgentGrant, subscribeBrowserAgentGrant } from "@/features/browser-agent/lib/api";
import { buildBrowserSessionCard } from "@/features/browsers/lib/addBrowserSession";
import { disposeBrowserSession } from "@/features/browsers/lib/disposeBrowserSession";
import { mainTabSid } from "@/features/playground/lib/browserGroups";
import {
  addPlaygroundSession,
  getPlaygroundStore,
  listPlaygroundBrowsers,
  showPlaygroundSession,
  subscribePlayground,
} from "@/features/playground/lib/sessions";
import { isPlaygroundWebviewOpen, screenshotPlaygroundWebview } from "@/features/playground/lib/webview";
import type { BrowserDeps } from "./commandHandler";
import type { BrowserSessionInfo } from "./protocol";
import { BrowserSessionManager, buildDesktopSessionList, fitImage, type DesktopBrowserRow, type Encoded } from "./browserSessions";

export const remoteBrowserManager = new BrowserSessionManager();

/** Everything the bridge needs from the desktop's Browsers page; injectable for tests. */
export type BrowserHost = {
  rows(): DesktopBrowserRow[];
  subscribe(fn: () => void): () => void;
  add(url: string, name: string): { sid: string; title: string };
  show(sid: string): void;
  remove(sid: string): Promise<void>;
  isOpen(sid: string): Promise<boolean>;
  screenshot(sid: string): Promise<{ bytes: number[] }>;
  /** sid -> current agent grant (surfaceId is the active tab; matched by any tab of the browser). */
  grants(): Promise<Array<{ surfaceId: string; agentPubkey: string; mode: "observe" | "drive"; webviewLabel: string }>>;
  setAgent(input: { sid: string; agentPubkey: string; channelId: string; mode: "observe" | "drive" }): Promise<void>;
  clearAgent(sid: string): Promise<void>;
  subscribeGrants(fn: () => void): () => void;
};

export const playgroundBrowserHost: BrowserHost = {
  rows() {
    const { sessions, browsers } = getPlaygroundStore();
    const out: DesktopBrowserRow[] = [];
    if (browsers.size === 0) {
      // Orphan sessions render as one-tab browsers on the Browsers page too.
      for (const m of sessions.values()) out.push({ sid: m.sid, title: m.name, url: m.url });
      return out;
    }
    for (const b of browsers.values()) {
      const main = sessions.get(mainTabSid(b)) ?? sessions.get(b.tabSids[0] ?? "");
      if (main) out.push({ sid: main.sid, title: main.name, url: main.url });
    }
    return out;
  },
  subscribe: (fn) => subscribePlayground(fn),
  add(url, name) {
    const card = buildBrowserSessionCard({ url, name });
    if (!card) throw new Error("That link can't be opened.");
    addPlaygroundSession(card, { preferSidePanel: true });
    showPlaygroundSession(card.sid, { preferSidePanel: true });
    return { sid: card.sid, title: card.name };
  },
  show: (sid) => showPlaygroundSession(sid, { preferSidePanel: true }),
  async remove(sid) {
    const row = {
      key: `browser:${sid}`,
      browserId: "",
      surfaceId: sid,
      mainSurfaceId: sid,
      title: "",
      url: "",
      host: "main" as const,
      windowLabel: "main",
      tabCount: 1,
      secondaryTabs: [],
    };
    const b = listPlaygroundBrowsers().find((x) => x.tabSids.includes(sid));
    if (b) row.browserId = b.browserId;
    const grants = await listBrowserAgentGrants().catch(() => []);
    await disposeBrowserSession({ row, grants, isEmbeddedLabel: () => false });
  },
  isOpen: (sid) => isPlaygroundWebviewOpen(sid),
  screenshot: (sid) => screenshotPlaygroundWebview(sid),
  grants: () => listBrowserAgentGrants().catch(() => []),
  async setAgent(i) {
    await setBrowserAgentGrant({
      surface: "playground",
      surfaceId: i.sid,
      agentId: i.agentPubkey,
      agentPubkey: i.agentPubkey,
      channelId: i.channelId,
      mode: i.mode,
      allowReplace: true,
    });
  },
  async clearAgent(sid) {
    const gs = await listBrowserAgentGrants().catch(() => []);
    for (const g of gs) if (g.surfaceId === sid) await clearBrowserAgentGrant(g.webviewLabel);
  },
  subscribeGrants(fn) {
    const p = subscribeBrowserAgentGrant(fn);
    return () => void p.then((u) => u());
  },
};

/** Current published list for this host (async because liveness and grants are native queries). */
export async function snapshotBrowserSessions(
  host: BrowserHost,
  agentName: (pubkey: string) => string | undefined,
  now: number,
): Promise<BrowserSessionInfo[]> {
  const rows = host.rows();
  const open = new Map<string, boolean>();
  await Promise.all(rows.map(async (r) => open.set(r.sid, await host.isOpen(r.sid).catch(() => false))));
  const grants = await host.grants();
  return buildDesktopSessionList({
    rows,
    remoteIds: new Set(remoteBrowserManager.ids()),
    isOpen: (sid) => open.get(sid),
    agentOf: (sid) => {
      const g = grants.find((x) => x.surfaceId === sid);
      return g ? { pubkey: g.agentPubkey, name: agentName(g.agentPubkey) ?? g.agentPubkey.slice(0, 8), mode: g.mode } : undefined;
    },
    now,
  });
}

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
  const deps: BrowserDeps = {
    manager: remoteBrowserManager,
    publishShot,
    publishSessions,
    exists: (sessionId) => host().rows().some((r) => r.sid === sessionId),
    async open(url, hostname) {
      const r = host().add(url, hostname);
      return { sessionId: r.sid, title: r.title };
    },
    async close(sessionId) {
      await host().remove(sessionId);
    },
    async capture(sessionId) {
      // A "Cold" browser has no webview yet: bring it forward first so it starts, then wait for it to load.
      const wasOpen = await host().isOpen(sessionId).catch(() => false);
      host().show(sessionId);
      await sleep(wasOpen ? 600 : 2500);
      const r = await host().screenshot(sessionId);
      if (!r.bytes?.length) throw new Error("The page isn't open on the desktop right now.");
      const png = Uint8Array.from(r.bytes);
      const fit = await fitImage((w, q) => encodeJpeg(png, w, q));
      if (!fit) throw new Error("The screenshot is too large to send.");
      return { mime: "image/jpeg", w: fit.w, h: fit.h, bytes: fit.bytes };
    },
    async setAgent({ sessionId, mode, agentPubkey, channelId }) {
      if (mode === "off") await host().clearAgent(sessionId);
      else await host().setAgent({ sid: sessionId, agentPubkey: agentPubkey ?? "", channelId: channelId ?? "", mode });
    },
  };
  stopAllRemoteBrowserSessions = async () => {
    for (const id of remoteBrowserManager.ids()) {
      try {
        await deps.close(id);
      } finally {
        remoteBrowserManager.remove(id);
      }
    }
    await publishSessions();
  };
  return deps;
}
