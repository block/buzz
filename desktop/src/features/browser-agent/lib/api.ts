import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type {
  BrowserAgentGrant,
  DriveAction,
  DriveActionResult,
  GrantSetInput,
  ObservePollResult,
} from "./types";

function native(): boolean {
  return isTauri() || import.meta.env.MODE === "e2e";
}

function windowLabel(): string {
  if (!isTauri()) return "main";
  try {
    return getCurrentWindow().label || "main";
  } catch {
    return "main";
  }
}

export async function setBrowserAgentGrant(
  input: GrantSetInput,
): Promise<BrowserAgentGrant> {
  if (!native()) {
    return {
      webviewLabel: `${input.surface}-${input.surfaceId}`,
      surface: input.surface,
      surfaceId: input.surfaceId,
      agentId: input.agentId,
      agentPubkey: input.agentPubkey,
      channelId: input.channelId,
      threadRoot: input.threadRoot ?? null,
      mode: input.mode,
      userHasControl: false,
      createdAtMs: Date.now(),
    };
  }
  return invoke<BrowserAgentGrant>("browser_agent_grant_set", {
    input: {
      ...input,
      windowLabel: input.windowLabel ?? windowLabel(),
    },
  });
}

export async function clearBrowserAgentGrant(
  webviewLabel: string,
): Promise<void> {
  if (!native()) return;
  await invoke("browser_agent_grant_clear", { webviewLabel });
}

export async function getBrowserAgentGrant(
  webviewLabel: string,
): Promise<BrowserAgentGrant | null> {
  if (!native()) return null;
  return invoke<BrowserAgentGrant | null>("browser_agent_grant_get", {
    webviewLabel,
  });
}

export async function listBrowserAgentGrants(): Promise<BrowserAgentGrant[]> {
  if (!native()) return [];
  return invoke<BrowserAgentGrant[]>("browser_agent_grants_list");
}

export async function takeBrowserAgentControl(
  webviewLabel: string,
): Promise<BrowserAgentGrant> {
  if (!native()) {
    return {
      webviewLabel,
      surface: "playground",
      surfaceId: "local",
      agentId: "",
      agentPubkey: "",
      channelId: "",
      threadRoot: null,
      mode: "drive",
      userHasControl: true,
      createdAtMs: Date.now(),
    };
  }
  return invoke<BrowserAgentGrant>("browser_agent_take_control", {
    webviewLabel,
  });
}

export async function releaseBrowserAgentControl(
  webviewLabel: string,
): Promise<BrowserAgentGrant> {
  if (!native()) {
    return {
      webviewLabel,
      surface: "playground",
      surfaceId: "local",
      agentId: "",
      agentPubkey: "",
      channelId: "",
      threadRoot: null,
      mode: "drive",
      userHasControl: false,
      createdAtMs: Date.now(),
    };
  }
  return invoke<BrowserAgentGrant>("browser_agent_release_control", {
    webviewLabel,
  });
}

export async function pollBrowserObserve(input: {
  webviewLabel: string;
  agentPubkey: string;
  afterId?: number;
  limit?: number;
}): Promise<ObservePollResult> {
  if (!native()) {
    throw new Error("browser observe requires native runtime");
  }
  return invoke<ObservePollResult>("browser_observe_poll", {
    input: {
      webviewLabel: input.webviewLabel,
      agentPubkey: input.agentPubkey,
      afterId: input.afterId ?? 0,
      limit: input.limit ?? 50,
    },
  });
}

export async function browserDrive(input: {
  webviewLabel: string;
  agentPubkey: string;
  action: DriveAction;
}): Promise<DriveActionResult> {
  if (!native()) {
    throw new Error("browser drive requires native runtime");
  }
  return invoke<DriveActionResult>("browser_drive", {
    input: {
      webviewLabel: input.webviewLabel,
      agentPubkey: input.agentPubkey,
      action: input.action,
      windowLabel: windowLabel(),
    },
  });
}


/**
 * Move a group grant onto another tab (tab switch / window.open focus).
 * Mirrors detach rebind: updates surfaceId + webviewLabel, reinstalls Drive lock.
 */
export async function rebindBrowserAgentGrantToTab(input: {
  fromSurfaceId: string;
  toSurfaceId: string;
  toWebviewLabel: string;
  fromWebviewLabel?: string;
}): Promise<BrowserAgentGrant | null> {
  if (!native()) return null;
  return invoke<BrowserAgentGrant | null>("browser_agent_rebind_surface", {
    input: {
      fromSurfaceId: input.fromSurfaceId,
      toSurfaceId: input.toSurfaceId,
      toWebviewLabel: input.toWebviewLabel,
      fromWebviewLabel: input.fromWebviewLabel ?? null,
    },
  });
}

/**
 * Copy Observe onto a new tab when the opener group already has a grant.
 * Drive stays on the active tab via rebind; this seeds Observe on siblings.
 */
export async function inheritObserveGrantToTab(input: {
  fromWebviewLabel: string;
  toSurfaceId: string;
  toWebviewLabel: string;
}): Promise<BrowserAgentGrant | null> {
  if (!native()) return null;
  const existing = await getBrowserAgentGrant(input.fromWebviewLabel);
  if (!existing) return null;
  return setBrowserAgentGrant({
    surface: "playground",
    surfaceId: input.toSurfaceId,
    agentId: existing.agentId,
    agentPubkey: existing.agentPubkey,
    channelId: existing.channelId,
    threadRoot: existing.threadRoot ?? null,
    mode: "observe",
    allowReplace: true,
  });
}

export function subscribeBrowserAgentGrant(
  onChange: (payload: {
    webviewLabel: string;
    grant: BrowserAgentGrant | null;
  }) => void,
): Promise<() => void> {
  if (!native()) return Promise.resolve(() => undefined);
  return listen<{ webviewLabel: string; grant: BrowserAgentGrant | null }>(
    "browser-agent-grant",
    (event) => onChange(event.payload),
  );
}

export type BrowserAgentObserveEvent = {
  webviewLabel: string;
  kind: string;
  payload?: unknown;
};

export function subscribeBrowserAgentObserve(
  onEvent: (payload: BrowserAgentObserveEvent) => void,
): Promise<() => void> {
  if (!native()) return Promise.resolve(() => undefined);
  return listen<BrowserAgentObserveEvent>(
    "browser-agent-observe",
    (event) => onEvent(event.payload),
  );
}

export type BrowserAgentTabInfo = {
  surfaceId: string;
  url: string;
  title: string;
  isMain: boolean;
};

export type BrowserAgentTabsSyncInput = {
  browserId: string;
  mainTabSid: string;
  activeTabSid: string;
  tabs: BrowserAgentTabInfo[];
  event?: {
    kind: "tab_opened" | "tab_switched";
    surfaceId: string;
    openerSurfaceId?: string;
    url?: string;
  } | null;
};

/** Mirror browser-group tabs for MCP browser_tabs / observe tab events. */
export async function syncBrowserAgentTabs(
  input: BrowserAgentTabsSyncInput,
): Promise<void> {
  if (!native()) return;
  await invoke("browser_agent_sync_tabs", { input });
}

/** Agent requested focus on a tab (MCP browser_switch_tab). */
export function subscribeBrowserAgentSwitchTab(
  onRequest: (payload: { surfaceId: string; browserId?: string }) => void,
): Promise<() => void> {
  if (!native()) return Promise.resolve(() => undefined);
  return listen<{ surfaceId: string; browserId?: string }>(
    "browser-agent-switch-tab",
    (event) => onRequest(event.payload),
  );
}

export type BrowserAgentRunbookMirror = {
  agentBrief: string;
  procedures: Array<{ id: string; title: string; summary: string }>;
  driveProtocol?: string[];
};

export type BrowserAgentRunbookFull = {
  agentBrief: string;
  procedures: Array<{
    id: string;
    title: string;
    steps: string;
    status: string;
    sourceAgent?: string;
    sourceChannel?: string;
    createdAt: number;
    updatedAt: number;
    acceptedAt?: number;
  }>;
  updatedAt: number;
};

/** Mirror runbook inject + full copy next to the live grant for MCP tools. */
export async function mirrorBrowserAgentRunbook(input: {
  webviewLabel: string;
  inject: BrowserAgentRunbookMirror;
  full: BrowserAgentRunbookFull;
}): Promise<void> {
  if (!native()) return;
  await invoke("browser_agent_mirror_runbook", {
    input: {
      webviewLabel: input.webviewLabel,
      inject: input.inject,
      full: input.full,
    },
  });
}

export type BrowserAgentRunbookPropose = {
  title: string;
  steps: string;
  sourceAgent?: string;
  sourceChannel?: string;
  atMs?: number;
};

/** Drain agent runbook proposals written by MCP next to the grant. */
export async function takeBrowserAgentRunbookProposes(
  webviewLabel: string,
): Promise<BrowserAgentRunbookPropose[]> {
  if (!native()) return [];
  return invoke<BrowserAgentRunbookPropose[]>(
    "browser_agent_take_runbook_proposes",
    { webviewLabel },
  );
}
