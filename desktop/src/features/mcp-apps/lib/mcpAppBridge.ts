import {
  AppBridge,
  type McpUiHostContext,
  type McpUiResourceCsp,
} from "@modelcontextprotocol/ext-apps/app-bridge";
import type {
  Transport,
  TransportSendOptions,
} from "@modelcontextprotocol/sdk/shared/transport.js";
import type {
  CallToolResult,
  JSONRPCMessage,
  ListResourcesResult,
  MessageExtraInfo,
  ReadResourceResult,
} from "@modelcontextprotocol/sdk/types.js";
import { JSONRPCMessageSchema } from "@modelcontextprotocol/sdk/types.js";

import {
  callMcpAppTool,
  listMcpAppResources,
  readMcpAppResource,
  type McpAppResource,
} from "@/shared/api/tauriMcpApps";

const HOST_INFO = { name: "Buzz Desktop", version: "1.0.0" };

export function mcpAppSandboxOrigin(sandboxUrl: string): string {
  const url = new URL(sandboxUrl);
  if (url.origin !== "null") return url.origin;
  if (!url.host) throw new Error("MCP App sandbox URL has no origin");
  return `${url.protocol}//${url.host}`;
}

class OriginValidatedPostMessageTransport implements Transport {
  private readonly eventSource: MessageEventSource;
  private readonly eventTarget: Window;
  private readonly expectedOrigin: string;
  private readonly messageListener: (event: MessageEvent) => void;

  constructor(
    eventTarget: Window,
    eventSource: MessageEventSource,
    expectedOrigin: string,
  ) {
    this.eventTarget = eventTarget;
    this.eventSource = eventSource;
    this.expectedOrigin = expectedOrigin;
    this.messageListener = (event) => {
      if (
        event.source !== this.eventSource ||
        event.origin !== this.expectedOrigin
      ) {
        return;
      }
      const parsed = JSONRPCMessageSchema.safeParse(event.data);
      if (parsed.success) {
        this.onmessage?.(parsed.data);
      } else if (event.data?.jsonrpc === "2.0") {
        this.onerror?.(
          new Error(`Invalid MCP App message: ${parsed.error.message}`),
        );
      }
    };
  }

  async start(): Promise<void> {
    window.addEventListener("message", this.messageListener);
  }

  async send(
    message: JSONRPCMessage,
    _options?: TransportSendOptions,
  ): Promise<void> {
    this.eventTarget.postMessage(message, this.expectedOrigin);
  }

  async close(): Promise<void> {
    window.removeEventListener("message", this.messageListener);
    this.onclose?.();
  }

  onclose?: () => void;
  onerror?: (error: Error) => void;
  onmessage?: (message: JSONRPCMessage, extra?: MessageExtraInfo) => void;
}

export type McpAppMessage = {
  role: "user";
  content: unknown;
};

export type McpAppModelContext = {
  content?: unknown[];
  structuredContent?: Record<string, unknown>;
};

export type McpAppBridgeCallbacks = {
  onMessage?: (message: McpAppMessage) => Promise<void> | void;
  onModelContext?: (context: McpAppModelContext | null) => Promise<void> | void;
  onOpenLink?: (url: string) => Promise<boolean> | boolean;
  onDisplayMode?: (mode: "inline" | "fullscreen") => void;
  onSizeChange?: (size: { width?: number; height?: number }) => void;
};

function bridgeResource(resource: McpAppResource) {
  return {
    uri: resource.uri,
    name: resource.name ?? resource.uri,
    title: resource.title ?? undefined,
    description: resource.description ?? undefined,
    mimeType: resource.mimeType ?? undefined,
    _meta: resource.meta,
  };
}

export function defaultMcpAppHostContext(): McpUiHostContext {
  return {
    theme: document.documentElement.classList.contains("dark")
      ? "dark"
      : "light",
    platform: "desktop",
    locale: navigator.language,
    timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    displayMode: "inline",
    availableDisplayModes: ["inline", "fullscreen"],
    containerDimensions: { maxHeight: 6000 },
    deviceCapabilities: {
      touch: navigator.maxTouchPoints > 0,
      hover: window.matchMedia("(hover: hover)").matches,
    },
  };
}

export function createMcpAppBridge(
  serverId: string,
  callbacks: McpAppBridgeCallbacks,
): AppBridge {
  const bridge = new AppBridge(
    null,
    HOST_INFO,
    {
      serverTools: {},
      serverResources: {},
      ...(callbacks.onMessage ? { message: { text: {} } } : {}),
      ...(callbacks.onModelContext
        ? { updateModelContext: { text: {}, structuredContent: {} } }
        : {}),
      ...(callbacks.onOpenLink ? { openLinks: {} } : {}),
    },
    { hostContext: defaultMcpAppHostContext() },
  );

  bridge.oncalltool = async ({ name, arguments: args }) =>
    (await callMcpAppTool(
      serverId,
      name,
      (args ?? {}) as Record<string, unknown>,
      "app",
    )) as CallToolResult;
  bridge.onlistresources = async () =>
    ({
      resources: (await listMcpAppResources(serverId)).map(bridgeResource),
    }) as ListResourcesResult;
  bridge.onreadresource = async ({ uri }) =>
    (await readMcpAppResource(serverId, uri)) as ReadResourceResult;
  bridge.onmessage = async (message) => {
    try {
      await callbacks.onMessage?.(message as McpAppMessage);
      return {};
    } catch {
      return { isError: true };
    }
  };
  bridge.onupdatemodelcontext = async (context) => {
    const hasContent = Boolean(context.content?.length);
    const hasStructured = Boolean(
      context.structuredContent &&
        Object.keys(context.structuredContent).length > 0,
    );
    await callbacks.onModelContext?.(
      hasContent || hasStructured ? (context as McpAppModelContext) : null,
    );
    return {};
  };
  if (callbacks.onOpenLink) {
    bridge.onopenlink = async ({ url }) => ({
      isError: !(await callbacks.onOpenLink?.(url)),
    });
  }
  bridge.onrequestdisplaymode = async ({ mode }) => {
    const next = mode === "fullscreen" ? "fullscreen" : "inline";
    callbacks.onDisplayMode?.(next);
    await bridge.sendHostContextChange({ displayMode: next });
    return { mode: next };
  };
  bridge.onsizechange = callbacks.onSizeChange;

  return bridge;
}

export async function connectMcpAppBridge(
  bridge: AppBridge,
  iframe: HTMLIFrameElement,
  expectedOrigin: string,
): Promise<void> {
  const targetWindow = iframe.contentWindow;
  if (!targetWindow) {
    throw new Error("MCP App sandbox browsing context is unavailable");
  }
  await bridge.connect(
    new OriginValidatedPostMessageTransport(
      targetWindow,
      targetWindow,
      expectedOrigin,
    ),
  );
}

export function observeMcpAppHostContext(
  bridge: AppBridge,
  iframe: HTMLIFrameElement,
): () => void {
  const sendDimensions = () => {
    const width = Math.round(iframe.getBoundingClientRect().width);
    if (width > 0) {
      void bridge.sendHostContextChange({
        containerDimensions: { width, maxHeight: 6000 },
      });
    }
  };
  const resizeObserver = new ResizeObserver(sendDimensions);
  resizeObserver.observe(iframe);

  const themeObserver = new MutationObserver(() => {
    void bridge.sendHostContextChange({
      theme: document.documentElement.classList.contains("dark")
        ? "dark"
        : "light",
    });
  });
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["class"],
  });
  sendDimensions();

  return () => {
    resizeObserver.disconnect();
    themeObserver.disconnect();
  };
}

export type PreparedBridgeResource = {
  html: string;
  csp: McpUiResourceCsp;
};
