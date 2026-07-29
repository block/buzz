import { mcpAppDisplayLabel } from "@/features/mcp-apps/lib/mcpAppMessage";
import type {
  McpAppResourceCsp,
  McpAppResourcePermissions,
  McpAppResourcePolicy,
} from "@/shared/api/tauriMcpApps";

const STORAGE_KEY_PREFIX = "buzz-channel-mcp-apps.v1";

export const CHANNEL_MCP_APPS_CHANGE_EVENT = "buzz:channel-mcp-apps-change";

export type ChannelMcpAppInstallation = {
  id: string;
  endpoint: string;
  serverName: string;
  toolName: string;
  title: string;
  resourceUri: string;
  arguments: Record<string, unknown>;
  approvedPolicy: McpAppResourcePolicy;
};

type ChannelMcpAppStore = {
  version: 1;
  channels: Record<string, ChannelMcpAppInstallation[]>;
};

const EMPTY_STORE: ChannelMcpAppStore = {
  version: 1,
  channels: {},
};

function stringList(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string")
    : [];
}

function normalizeCsp(value: unknown): McpAppResourceCsp {
  const source =
    value && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : {};
  return {
    connectDomains: stringList(source.connectDomains),
    resourceDomains: stringList(source.resourceDomains),
    frameDomains: stringList(source.frameDomains),
    baseUriDomains: stringList(source.baseUriDomains),
  };
}

function normalizePermissions(value: unknown): McpAppResourcePermissions {
  const source =
    value && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : {};
  const requested = (name: string) =>
    source[name] && typeof source[name] === "object" ? {} : undefined;
  return {
    camera: requested("camera"),
    microphone: requested("microphone"),
    geolocation: requested("geolocation"),
    clipboardWrite: requested("clipboardWrite"),
  };
}

function normalizePolicy(value: unknown): McpAppResourcePolicy {
  const source =
    value && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : {};
  return {
    csp: normalizeCsp(source.csp),
    requestedPermissions: normalizePermissions(source.requestedPermissions),
  };
}

function storageKey(pubkey: string): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

function normalizeInstallation(
  value: unknown,
): ChannelMcpAppInstallation | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  const id = typeof source.id === "string" ? source.id.trim() : "";
  const endpoint =
    typeof source.endpoint === "string" ? source.endpoint.trim() : "";
  const toolName =
    typeof source.toolName === "string" ? source.toolName.trim() : "";
  const resourceUri =
    typeof source.resourceUri === "string" ? source.resourceUri.trim() : "";
  if (!id || !endpoint || !toolName || !resourceUri.startsWith("ui://")) {
    return null;
  }
  const argumentsValue =
    source.arguments &&
    typeof source.arguments === "object" &&
    !Array.isArray(source.arguments)
      ? (source.arguments as Record<string, unknown>)
      : {};
  return {
    id,
    endpoint,
    serverName:
      typeof source.serverName === "string" && source.serverName.trim()
        ? mcpAppDisplayLabel(source.serverName, endpoint, 120)
        : endpoint,
    toolName,
    title:
      typeof source.title === "string" && source.title.trim()
        ? mcpAppDisplayLabel(source.title, toolName)
        : mcpAppDisplayLabel(toolName, "Channel app"),
    resourceUri,
    arguments: argumentsValue,
    approvedPolicy: normalizePolicy(source.approvedPolicy),
  };
}

export function parseChannelMcpAppStore(
  value: unknown,
): ChannelMcpAppStore | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const source = value as Record<string, unknown>;
  if (
    source.version !== 1 ||
    !source.channels ||
    typeof source.channels !== "object" ||
    Array.isArray(source.channels)
  ) {
    return null;
  }
  const channels = Object.fromEntries(
    Object.entries(source.channels as Record<string, unknown>).flatMap(
      ([channelId, installations]) => {
        if (!Array.isArray(installations)) return [];
        const normalized = installations
          .map(normalizeInstallation)
          .filter(
            (installation): installation is ChannelMcpAppInstallation =>
              installation !== null,
          );
        return normalized.length > 0 ? [[channelId, normalized]] : [];
      },
    ),
  );
  return { version: 1, channels };
}

function readStore(pubkey: string): ChannelMcpAppStore {
  if (typeof window === "undefined") return EMPTY_STORE;
  try {
    const raw = window.localStorage.getItem(storageKey(pubkey));
    if (!raw) return EMPTY_STORE;
    return parseChannelMcpAppStore(JSON.parse(raw)) ?? EMPTY_STORE;
  } catch {
    return EMPTY_STORE;
  }
}

function notify(): void {
  if (
    typeof window !== "undefined" &&
    typeof window.dispatchEvent === "function"
  ) {
    window.dispatchEvent(new CustomEvent(CHANNEL_MCP_APPS_CHANGE_EVENT));
  }
}

function writeStore(pubkey: string, store: ChannelMcpAppStore): boolean {
  if (typeof window === "undefined") return false;
  try {
    window.localStorage.setItem(storageKey(pubkey), JSON.stringify(store));
    notify();
    return true;
  } catch {
    return false;
  }
}

export function subscribeChannelMcpApps(listener: () => void): () => void {
  if (typeof window === "undefined") return () => {};
  const onChange = () => listener();
  const onStorage = (event: StorageEvent) => {
    if (event.key === null || event.key.startsWith(STORAGE_KEY_PREFIX)) {
      listener();
    }
  };
  window.addEventListener(CHANNEL_MCP_APPS_CHANGE_EVENT, onChange);
  window.addEventListener("storage", onStorage);
  return () => {
    window.removeEventListener(CHANNEL_MCP_APPS_CHANGE_EVENT, onChange);
    window.removeEventListener("storage", onStorage);
  };
}

export function getChannelMcpApps(
  pubkey: string,
  channelId: string,
): ChannelMcpAppInstallation[] {
  return [...(readStore(pubkey).channels[channelId] ?? [])];
}

export function installChannelMcpApp(
  pubkey: string,
  channelId: string,
  installation: ChannelMcpAppInstallation,
): boolean {
  const store = readStore(pubkey);
  const current = store.channels[channelId] ?? [];
  const next = [
    ...current.filter((candidate) => candidate.id !== installation.id),
    installation,
  ];
  return writeStore(pubkey, {
    version: 1,
    channels: { ...store.channels, [channelId]: next },
  });
}

export function removeChannelMcpApp(
  pubkey: string,
  channelId: string,
  installationId: string,
): boolean {
  const store = readStore(pubkey);
  const current = store.channels[channelId] ?? [];
  const next = current.filter(
    (installation) => installation.id !== installationId,
  );
  if (next.length === current.length) return true;
  const channels = { ...store.channels };
  if (next.length > 0) {
    channels[channelId] = next;
  } else {
    delete channels[channelId];
  }
  return writeStore(pubkey, { version: 1, channels });
}
