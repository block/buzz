import type { Workspace } from "./types";

const WORKSPACES_KEY = "sprout-workspaces";
const ACTIVE_WORKSPACE_KEY = "sprout-active-workspace-id";

export function loadWorkspaces(): Workspace[] {
  try {
    const raw = localStorage.getItem(WORKSPACES_KEY);
    if (!raw) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return [];
    }
    // Migration: older builds stored the user's `nsec` in localStorage and
    // re-applied it to the backend on every reload, which silently overwrote
    // any `import_identity` result with the original generated key. The
    // on-disk `identity.key` file is the only source of truth now. Strip
    // any lingering `nsec` from existing entries on read and persist the
    // cleaned list back so it cannot leak into future sessions.
    let didStrip = false;
    const cleaned = (parsed as Array<Record<string, unknown>>).map((entry) => {
      if (entry && typeof entry === "object" && "nsec" in entry) {
        const { nsec: _nsec, ...rest } = entry;
        didStrip = true;
        return rest;
      }
      return entry;
    }) as Workspace[];
    if (didStrip) {
      localStorage.setItem(WORKSPACES_KEY, JSON.stringify(cleaned));
    }
    return cleaned;
  } catch {
    return [];
  }
}

export function saveWorkspaces(workspaces: Workspace[]): void {
  localStorage.setItem(WORKSPACES_KEY, JSON.stringify(workspaces));
}

export function clearWorkspaceStorage(): void {
  localStorage.removeItem(WORKSPACES_KEY);
  localStorage.removeItem(ACTIVE_WORKSPACE_KEY);
}

export function loadActiveWorkspaceId(): string | null {
  return localStorage.getItem(ACTIVE_WORKSPACE_KEY);
}

export function saveActiveWorkspaceId(id: string): void {
  localStorage.setItem(ACTIVE_WORKSPACE_KEY, id);
}

export function normalizeRelayUrl(url: string): string {
  if (!url.startsWith("ws://") && !url.startsWith("wss://")) {
    return `wss://${url}`;
  }
  return url;
}

export function deriveWorkspaceName(relayUrl: string): string {
  try {
    const url = new URL(
      relayUrl.replace("ws://", "http://").replace("wss://", "https://"),
    );
    const host = url.hostname;
    if (host === "localhost" || host === "127.0.0.1") {
      return "Local Dev";
    }
    const parts = host.split(".");
    // Detect staging environments (e.g. sprout-oss.stage.blox.sqprod.co)
    if (parts.some((p) => p === "stage" || p === "staging")) {
      return "Sprout (staging)";
    }
    // Use the first subdomain segment or the domain itself
    if (parts.length >= 2) {
      return parts[0] === "relay" ? parts[1] : parts[0];
    }
    return host;
  } catch {
    return "Workspace";
  }
}

export function initFirstWorkspace(
  relayUrl: string,
  pubkey: string,
  name?: string,
): Workspace {
  const normalizedUrl = normalizeRelayUrl(relayUrl);
  const trimmedName = name?.trim();
  const workspace: Workspace = {
    id: crypto.randomUUID(),
    name: trimmedName || deriveWorkspaceName(normalizedUrl),
    relayUrl: normalizedUrl,
    pubkey,
    addedAt: new Date().toISOString(),
  };
  saveWorkspaces([workspace]);
  saveActiveWorkspaceId(workspace.id);
  return workspace;
}
